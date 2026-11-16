use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gitpoulpe_core::{DiffLine, DiffLineKind};

pub const SEGMENT_MAX_LINES: usize = 1024;
pub const SEGMENT_MAX_BYTES: usize = 256 * 1024;
pub const DEFAULT_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_DISK_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub dir: PathBuf,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}

impl CacheConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            memory_bytes: DEFAULT_MEMORY_BYTES,
            disk_bytes: DEFAULT_DISK_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub segments: usize,
}

pub(crate) enum SegmentRead {
    Memory(Arc<Vec<u8>>),
    Disk(PathBuf),
    Absent,
}

struct Segment {
    memory: Option<Arc<Vec<u8>>>,
    on_disk: bool,
    size: u64,
    tick: u64,
}

#[derive(Default)]
struct Inner {
    tick: u64,
    memory_bytes: u64,
    disk_bytes: u64,
    segments: HashMap<(u64, u32), Segment>,
}

struct Shared {
    config: CacheConfig,
    root: PathBuf,
    next_document: AtomicU64,
    inner: Mutex<Inner>,
}

#[derive(Clone)]
pub struct DiffCache {
    shared: Arc<Shared>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn verify_private_directory(dir: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |reason: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            reason.to_owned(),
        ))
    };
    if !meta.file_type().is_dir() {
        return refuse("the cache directory is not a real directory");
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return refuse("the cache directory is accessible to other users");
    }
    Ok(())
}

fn pid_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

impl DiffCache {
    pub fn open(config: CacheConfig) -> io::Result<Self> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&config.dir)?;
        verify_private_directory(&config.dir)?;
        Self::remove_interrupted_runs(&config.dir);
        static RUN: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "run-{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        );
        let root = config.dir.join(name);
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let base_owner = std::fs::symlink_metadata(&config.dir)?.uid();
        if std::fs::symlink_metadata(&root)?.uid() != base_owner {
            let _ = std::fs::remove_dir(&root);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the cache directory belongs to another user",
            ));
        }
        let root = std::fs::canonicalize(root)?;
        Ok(Self {
            shared: Arc::new(Shared {
                config,
                root,
                next_document: AtomicU64::new(1),
                inner: Mutex::new(Inner::default()),
            }),
        })
    }

    pub fn default_dir() -> PathBuf {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("gitpoulpe").join("diff")
    }

    fn remove_interrupted_runs(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name.to_str().and_then(|n| n.strip_prefix("run-")) else {
                continue;
            };
            let Some(pid) = rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            if pid != std::process::id() && !pid_is_alive(pid) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.shared.inner.lock().unwrap();
        CacheStats {
            memory_bytes: inner.memory_bytes,
            disk_bytes: inner.disk_bytes,
            segments: inner.segments.len(),
        }
    }

    pub(crate) fn new_document(&self) -> u64 {
        self.shared.next_document.fetch_add(1, Ordering::Relaxed)
    }

    fn path(&self, document: u64, index: u32) -> PathBuf {
        self.shared.root.join(format!("{document}-{index}.seg"))
    }

    pub(crate) fn insert(&self, document: u64, index: u32, bytes: Vec<u8>) {
        let size = bytes.len() as u64;
        let mut inner = self.shared.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        if let Some(old) = inner.segments.remove(&(document, index)) {
            self.release(&mut inner, document, index, &old);
        }
        inner.memory_bytes += size;
        inner.segments.insert(
            (document, index),
            Segment {
                memory: Some(Arc::new(bytes)),
                on_disk: false,
                size,
                tick,
            },
        );
        self.enforce(&mut inner);
    }

    pub(crate) fn get(&self, document: u64, index: u32) -> SegmentRead {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.tick += 1;
        let tick = inner.tick;
        match inner.segments.get_mut(&(document, index)) {
            Some(segment) => {
                segment.tick = tick;
                match &segment.memory {
                    Some(bytes) => SegmentRead::Memory(bytes.clone()),
                    None => SegmentRead::Disk(self.path(document, index)),
                }
            }
            None => SegmentRead::Absent,
        }
    }

    pub(crate) fn forget(&self, document: u64, index: u32) {
        let mut inner = self.shared.inner.lock().unwrap();
        if let Some(segment) = inner.segments.remove(&(document, index)) {
            self.release(&mut inner, document, index, &segment);
        }
    }

    pub(crate) fn remove_document(&self, document: u64) {
        let mut inner = self.shared.inner.lock().unwrap();
        let keys: Vec<_> = inner
            .segments
            .keys()
            .filter(|(d, _)| *d == document)
            .copied()
            .collect();
        for key in keys {
            if let Some(segment) = inner.segments.remove(&key) {
                self.release(&mut inner, key.0, key.1, &segment);
            }
        }
    }

    fn release(&self, inner: &mut Inner, document: u64, index: u32, segment: &Segment) {
        if segment.memory.is_some() {
            inner.memory_bytes -= segment.size;
        }
        if segment.on_disk {
            inner.disk_bytes -= segment.size;
            let _ = std::fs::remove_file(self.path(document, index));
        }
    }

    fn enforce(&self, inner: &mut Inner) {
        while inner.memory_bytes > self.shared.config.memory_bytes {
            let Some(key) = Self::oldest(inner, |s| s.memory.is_some()) else {
                break;
            };
            let (size, on_disk) = {
                let segment = &inner.segments[&key];
                (segment.size, segment.on_disk)
            };
            let spilled = on_disk || self.spill(inner, key, size);
            let segment = inner.segments.get_mut(&key).expect("segment exists");
            segment.memory = None;
            segment.on_disk = spilled;
            inner.memory_bytes -= size;
            if !spilled {
                inner.segments.remove(&key);
            }
        }
        while inner.disk_bytes > self.shared.config.disk_bytes {
            let Some(key) = Self::oldest(inner, |s| s.on_disk) else {
                break;
            };
            let segment = inner.segments.get_mut(&key).expect("segment exists");
            segment.on_disk = false;
            inner.disk_bytes -= segment.size;
            let _ = std::fs::remove_file(self.path(key.0, key.1));
            if segment.memory.is_none() {
                inner.segments.remove(&key);
            }
        }
    }

    fn spill(&self, inner: &mut Inner, key: (u64, u32), size: u64) -> bool {
        let tick = inner.segments[&key].tick;
        while inner.disk_bytes + size > self.shared.config.disk_bytes {
            let Some(older) =
                Self::oldest(inner, |s| s.on_disk).filter(|k| inner.segments[k].tick < tick)
            else {
                return false;
            };
            let segment = inner.segments.get_mut(&older).expect("segment exists");
            segment.on_disk = false;
            inner.disk_bytes -= segment.size;
            let _ = std::fs::remove_file(self.path(older.0, older.1));
            if segment.memory.is_none() {
                inner.segments.remove(&older);
            }
        }
        let Some(bytes) = inner.segments[&key].memory.clone() else {
            return false;
        };
        let path = self.path(key.0, key.1);
        let _ = std::fs::remove_file(&path);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(bytes.as_slice()));
        if written.is_err() {
            let _ = std::fs::remove_file(&path);
            return false;
        }
        inner.disk_bytes += size;
        true
    }

    fn oldest(inner: &Inner, filter: impl Fn(&Segment) -> bool) -> Option<(u64, u32)> {
        inner
            .segments
            .iter()
            .filter(|(_, s)| filter(s))
            .min_by_key(|(_, s)| s.tick)
            .map(|(k, _)| *k)
    }
}

pub(crate) fn checksum(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

pub(crate) fn encode(lines: &[DiffLine]) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines.iter().map(|l| l.text.len() + 13).sum());
    for line in lines {
        out.push(match line.kind {
            DiffLineKind::HunkHeader => 0,
            DiffLineKind::Context => 1,
            DiffLineKind::Added => 2,
            DiffLineKind::Removed => 3,
            DiffLineKind::NoNewlineAtEof => 4,
        });
        out.extend_from_slice(&line.old_number.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&line.new_number.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&(line.text.len() as u32).to_le_bytes());
        out.extend_from_slice(&line.text);
    }
    out
}

pub(crate) fn decode(mut bytes: &[u8]) -> Result<Vec<DiffLine>, String> {
    let mut lines = Vec::new();
    while !bytes.is_empty() {
        if bytes.len() < 13 {
            return Err("truncated segment".to_owned());
        }
        let kind = match bytes[0] {
            0 => DiffLineKind::HunkHeader,
            1 => DiffLineKind::Context,
            2 => DiffLineKind::Added,
            3 => DiffLineKind::Removed,
            4 => DiffLineKind::NoNewlineAtEof,
            other => return Err(format!("unknown line kind {other}")),
        };
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        let (old, new, len) = (word(1), word(5), word(9) as usize);
        if bytes.len() < 13 + len {
            return Err("truncated segment".to_owned());
        }
        lines.push(DiffLine {
            kind,
            old_number: (old != 0).then_some(old),
            new_number: (new != 0).then_some(new),
            text: bytes[13..13 + len].to_vec(),
        });
        bytes = &bytes[13 + len..];
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_tempdir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn cache(memory: u64, disk: u64) -> (tempfile::TempDir, DiffCache) {
        let dir = private_tempdir();
        let mut config = CacheConfig::new(dir.path());
        config.memory_bytes = memory;
        config.disk_bytes = disk;
        let cache = DiffCache::open(config).unwrap();
        (dir, cache)
    }

    #[test]
    fn lines_survive_an_encode_decode_round_trip() {
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::HunkHeader,
                old_number: None,
                new_number: None,
                text: b"@@ -1 +1 @@".to_vec(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                old_number: Some(1),
                new_number: Some(1),
                text: b"\xff\n\0".to_vec(),
            },
            DiffLine {
                kind: DiffLineKind::NoNewlineAtEof,
                old_number: None,
                new_number: None,
                text: Vec::new(),
            },
        ];
        assert_eq!(decode(&encode(&lines)).unwrap(), lines);
        assert!(decode(&encode(&lines)[..20]).is_err());
    }

    #[test]
    fn memory_overflow_spills_the_oldest_segments_to_disk_and_they_read_back() {
        let (_dir, cache) = cache(250, 10_000);
        for index in 0..4u32 {
            cache.insert(1, index, vec![index as u8; 100]);
        }
        let stats = cache.stats();
        assert!(
            stats.memory_bytes <= 250 && stats.disk_bytes > 0,
            "{stats:?}"
        );
        let SegmentRead::Disk(path) = cache.get(1, 0) else {
            panic!("oldest segment should be on disk")
        };
        assert_eq!(std::fs::read(path).unwrap(), vec![0u8; 100]);
        assert!(matches!(cache.get(1, 3), SegmentRead::Memory(_)));
    }

    #[test]
    fn disk_overflow_evicts_the_least_recently_used_segments_entirely() {
        let (_dir, cache) = cache(100, 250);
        for index in 0..8u32 {
            cache.insert(1, index, vec![0u8; 100]);
        }
        let stats = cache.stats();
        assert!(
            stats.memory_bytes <= 100 && stats.disk_bytes <= 250,
            "{stats:?}"
        );
        assert!(matches!(cache.get(1, 0), SegmentRead::Absent));
        assert!(matches!(cache.get(1, 7), SegmentRead::Memory(_)));
    }

    #[test]
    fn removing_a_document_frees_memory_disk_and_files() {
        let (_dir, cache) = cache(100, 10_000);
        for index in 0..5u32 {
            cache.insert(7, index, vec![0u8; 100]);
        }
        cache.insert(8, 0, vec![0u8; 50]);
        cache.remove_document(7);
        let stats = cache.stats();
        assert_eq!((stats.segments, stats.disk_bytes), (1, 0));
        assert!(std::fs::read_dir(cache.root()).unwrap().next().is_none());
    }

    #[test]
    fn the_run_directory_is_private_and_removed_on_drop_and_interrupted_runs_are_cleaned() {
        let dir = private_tempdir();
        let stale = dir.path().join("run-4194999-0");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("1-0.seg"), b"x").unwrap();
        let cache = DiffCache::open(CacheConfig::new(dir.path())).unwrap();
        assert!(!stale.exists(), "an interrupted run was not cleaned up");
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(cache.root()).unwrap().permissions(),
        );
        assert_eq!(mode & 0o777, 0o700);
        let root = cache.root().to_path_buf();
        drop(cache);
        assert!(!root.exists());
    }

    #[test]
    fn default_budgets_are_those_of_the_plan() {
        let config = CacheConfig::new("/nonexistent");
        assert_eq!(config.memory_bytes, 64 * 1024 * 1024);
        assert_eq!(config.disk_bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn a_cache_directory_that_is_a_symlink_or_open_to_others_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(DiffCache::open(CacheConfig::new(&link)).is_err());

        let open = base.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(DiffCache::open(CacheConfig::new(&open)).is_err());

        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(DiffCache::open(CacheConfig::new(&open)).is_ok());
    }

    #[test]
    fn spilled_segments_are_private_files_and_never_follow_a_planted_link() {
        let (_dir, cache) = cache(50, 10_000);
        let planted_target = tempfile::tempdir().unwrap().keep().join("victim");
        std::os::unix::fs::symlink(&planted_target, cache.root().join("1-0.seg")).unwrap();
        cache.insert(1, 0, vec![1u8; 100]);
        cache.insert(1, 1, vec![2u8; 100]);
        assert!(!planted_target.exists(), "a planted symlink was followed");
        let SegmentRead::Disk(path) = cache.get(1, 0) else {
            panic!("segment 0 should have spilled")
        };
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.file_type().is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
