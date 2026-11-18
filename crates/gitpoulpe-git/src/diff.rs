use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::ops::Range;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gitpoulpe_core::{
    BuildState, ChangeEntry, CommitRange, Diagnostic, DiffKey, DiffLine, DiffSide, DocumentKind,
    DocumentMeta, EntryKind, FileRef, Generation, Hunk, RangeData, RangeError, ReadError,
    SessionId, SessionInfo, SplitIndex, SplitIndexBuilder,
};
use tokio::sync::watch;

use crate::cache::{
    DiffCache, SEGMENT_MAX_BYTES, SEGMENT_MAX_LINES, SegmentRead, checksum, decode, encode,
};
use crate::diff_parse::DiffParser;
use crate::process::RunError;
use crate::{Cancel, Git};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffError {
    NoWorktree,
    NotInThisView,
    CacheInsideRepository,
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("the repository has no working directory"),
            Self::NotInThisView => f.write_str("the file has no changes in this view"),
            Self::CacheInsideRepository => {
                f.write_str("the diff cache directory is inside the repository")
            }
        }
    }
}

impl std::error::Error for DiffError {}

#[derive(Clone)]
struct Request {
    dir: PathBuf,
    side: DiffSide,
    path: PathBuf,
    original: Option<PathBuf>,
    untracked: bool,
    conflict: bool,
    range: Option<CommitRange>,
}

impl Request {
    fn args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
        ]
        .map(OsString::from)
        .into();
        let literal = |path: &PathBuf| {
            let mut spec = b":(literal)".to_vec();
            spec.extend_from_slice(path.as_os_str().as_bytes());
            OsString::from_vec(spec)
        };
        if let Some(range) = &self.range {
            args.push(OsString::from("-M"));
            args.push(OsString::from(range.base.clone()));
            args.push(OsString::from(range.oid.clone()));
            args.push(OsString::from("--"));
            args.push(literal(&self.path));
            if let Some(original) = &self.original {
                args.push(literal(original));
            }
            return args;
        }
        if self.untracked {
            args.extend(["--no-index", "--", "/dev/null"].map(OsString::from));
            args.push(self.path.clone().into_os_string());
            return args;
        }
        if self.side == DiffSide::Staged {
            args.extend(["--cached", "-M"].map(OsString::from));
        }
        args.push(OsString::from("--"));
        args.push(literal(&self.path));
        if self.side == DiffSide::Staged
            && let Some(original) = &self.original
        {
            args.push(literal(original));
        }
        args
    }
}

#[derive(Clone, Copy)]
struct SegmentMeta {
    start: u64,
    lines: u32,
    checksum: u64,
}

struct DocState {
    build: BuildState,
    kind: Option<DocumentKind>,
    lines: u64,
    added: u64,
    removed: u64,
    flushed_lines: u64,
    hunks: Vec<Hunk>,
    type_changed: bool,
    splitter: SplitIndexBuilder,
    split: Option<Arc<SplitIndex>>,
    segments: Vec<SegmentMeta>,
    tail: Vec<DiffLine>,
    tail_bytes: usize,
}

struct DocInner {
    key: DiffKey,
    id: u64,
    git: Git,
    cache: DiffCache,
    request: Request,
    cancel: Cancel,
    retired: AtomicBool,
    state: Mutex<DocState>,
    progress: watch::Sender<u64>,
}

#[derive(Clone)]
pub struct DiffDocument {
    inner: Arc<DocInner>,
}

fn unreadable(message: impl Into<String>) -> DocumentKind {
    DocumentKind::Unreadable(Diagnostic {
        exit_code: None,
        signal: None,
        message: message.into(),
        cause: None,
    })
}

impl DocInner {
    fn notify(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    fn append(&self, lines: &[DiffLine], hunks: &[Hunk], counts: (u64, u64)) {
        let mut state = self.state.lock().unwrap();
        for line in lines {
            state.tail_bytes += line.text.len() + 13;
            state.splitter.push(line.kind);
            state.tail.push(line.clone());
            state.lines += 1;
            if state.tail.len() >= SEGMENT_MAX_LINES || state.tail_bytes >= SEGMENT_MAX_BYTES {
                self.flush_tail(&mut state);
            }
        }
        state.hunks = hunks.to_vec();
        (state.added, state.removed) = counts;
        drop(state);
        self.notify();
    }

    fn flush_tail(&self, state: &mut DocState) {
        if state.tail.is_empty() {
            return;
        }
        let bytes = encode(&state.tail);
        let index = state.segments.len() as u32;
        state.segments.push(SegmentMeta {
            start: state.flushed_lines,
            lines: state.tail.len() as u32,
            checksum: checksum(&bytes),
        });
        state.flushed_lines += state.tail.len() as u64;
        state.tail.clear();
        state.tail_bytes = 0;
        self.cache.insert(self.id, index, bytes);
    }

    fn finish(&self, kind: DocumentKind, hunks: Option<Vec<Hunk>>, type_changed: bool) {
        let mut state = self.state.lock().unwrap();
        self.flush_tail(&mut state);
        if let Some(hunks) = hunks {
            state.hunks = hunks;
        }
        state.type_changed = type_changed;
        let splitter = std::mem::take(&mut state.splitter);
        state.split = (kind == DocumentKind::Text).then(|| Arc::new(splitter.finish()));
        state.kind = Some(kind);
        state.build = BuildState::Complete;
        drop(state);
        self.notify();
    }

    fn fail(&self, kind: DocumentKind) {
        self.cache.remove_document(self.id);
        let mut state = self.state.lock().unwrap();
        state.segments.clear();
        state.tail.clear();
        state.tail_bytes = 0;
        state.lines = 0;
        state.added = 0;
        state.removed = 0;
        state.flushed_lines = 0;
        state.hunks.clear();
        state.splitter = SplitIndexBuilder::new();
        state.split = None;
        state.kind = Some(kind);
        state.build = BuildState::Complete;
        drop(state);
        self.notify();
    }

    fn cancelled(&self) {
        self.cache.remove_document(self.id);
        let mut state = self.state.lock().unwrap();
        state.segments.clear();
        state.tail.clear();
        state.build = BuildState::Cancelled;
        drop(state);
        self.notify();
    }

    async fn environment(&self, cancel: &Cancel) -> Result<Vec<(OsString, OsString)>, ReadError> {
        self.git.filter_overrides(&self.request.dir, cancel).await
    }

    async fn build(self: Arc<Self>) {
        if self.request.conflict {
            self.finish(DocumentKind::Conflict, None, false);
            return;
        }
        let env = match self.environment(&self.cancel).await {
            Ok(env) => env,
            Err(ReadError::Cancelled) => return self.cancelled(),
            Err(error) => return self.fail(unreadable(error.to_string())),
        };
        let args = self.request.args();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut parser = DiffParser::new();
        let mut pending = Vec::new();
        let result = self
            .git
            .run_env(&self.request.dir, &refs, &env, &self.cancel, |chunk| {
                pending.clear();
                parser
                    .push(chunk, &mut pending)
                    .map_err(|e| e.to_string())?;
                self.append(&pending, parser.hunks(), (parser.added(), parser.removed()));
                Ok(())
            })
            .await;
        match result {
            Err(RunError::Cancelled) => self.cancelled(),
            Err(RunError::Spawn(e)) | Err(RunError::Io(e)) => {
                self.fail(DocumentKind::Unreadable(RunError::spawn_diagnostic(&e)))
            }
            Err(RunError::Rejected(message)) => {
                self.fail(unreadable(format!("unexpected diff output: {message}")))
            }
            Ok(run) => {
                let acceptable =
                    run.code == Some(0) || (run.code == Some(1) && self.request.untracked);
                if !acceptable {
                    return self.fail(DocumentKind::Unreadable(run.diagnostic()));
                }
                match parser.finish() {
                    Err(error) => self.fail(unreadable(format!("unexpected diff output: {error}"))),
                    Ok(summary) => {
                        let kind = if summary.binary {
                            DocumentKind::Binary
                        } else if !summary.hunks.is_empty() {
                            DocumentKind::Text
                        } else if let Some((old, new)) = summary.mode_change {
                            DocumentKind::ModeChange { old, new }
                        } else if let Some((from, to)) = summary.rename {
                            DocumentKind::RenameOnly { from, to }
                        } else {
                            DocumentKind::NoChanges
                        };
                        self.finish(kind, Some(summary.hunks), summary.type_changed);
                    }
                }
            }
        }
    }

    async fn rebuild_segment(
        &self,
        index: u32,
        meta: SegmentMeta,
        cancel: &Cancel,
    ) -> Result<Vec<DiffLine>, RangeError> {
        let env = self.environment(cancel).await.map_err(|e| match e {
            ReadError::Cancelled => RangeError::Cancelled,
            other => RangeError::Storage(other.to_string()),
        })?;
        let args = self.request.args();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let end = meta.start + u64::from(meta.lines);
        let mut parser = DiffParser::new();
        let mut scratch = Vec::new();
        let mut collected = Vec::with_capacity(meta.lines as usize);
        let mut position = 0u64;
        let result = self
            .git
            .run_env(&self.request.dir, &refs, &env, cancel, |chunk| {
                scratch.clear();
                parser
                    .push(chunk, &mut scratch)
                    .map_err(|e| e.to_string())?;
                for line in scratch.drain(..) {
                    if position >= meta.start && position < end {
                        collected.push(line);
                    }
                    position += 1;
                    if position >= end {
                        return Err("segment collected".to_owned());
                    }
                }
                Ok(())
            })
            .await;
        match result {
            Err(RunError::Cancelled) => return Err(RangeError::Cancelled),
            Err(RunError::Spawn(e)) | Err(RunError::Io(e)) => {
                return Err(RangeError::Storage(e.to_string()));
            }
            Ok(_) | Err(RunError::Rejected(_)) => {}
        }
        let bytes = encode(&collected);
        if collected.len() != meta.lines as usize || checksum(&bytes) != meta.checksum {
            return Err(RangeError::Stale);
        }
        if self.retired.load(Ordering::SeqCst) {
            return Err(RangeError::Stale);
        }
        self.cache.insert(self.id, index, bytes);
        Ok(collected)
    }

    async fn segment(
        &self,
        index: u32,
        meta: SegmentMeta,
        cancel: &Cancel,
    ) -> Result<Vec<DiffLine>, RangeError> {
        let cached = match self.cache.get(self.id, index) {
            SegmentRead::Memory(bytes) => Some(bytes.as_ref().clone()),
            SegmentRead::Disk(path) => std::fs::read(path).ok(),
            SegmentRead::Absent => None,
        };
        if let Some(bytes) = cached
            && checksum(&bytes) == meta.checksum
            && let Ok(lines) = decode(&bytes)
        {
            return Ok(lines);
        }
        self.cache.forget(self.id, index);
        self.rebuild_segment(index, meta, cancel).await
    }
}

impl DiffDocument {
    pub fn key(&self) -> &DiffKey {
        &self.inner.key
    }

    pub fn id(&self) -> u64 {
        self.inner.id
    }

    pub fn meta(&self) -> DocumentMeta {
        let state = self.inner.state.lock().unwrap();
        DocumentMeta {
            state: state.build,
            kind: state.kind.clone(),
            lines: state.lines,
            added: state.added,
            removed: state.removed,
            hunks: state.hunks.clone(),
            type_changed: state.type_changed,
            split: state.split.clone(),
        }
    }

    pub fn cancel(&self) {
        self.inner.cancel.cancel();
    }

    pub fn is_retired(&self) -> bool {
        self.inner.retired.load(Ordering::SeqCst)
    }

    pub async fn wait_finished(&self, cancel: &Cancel) -> Result<DocumentMeta, RangeError> {
        loop {
            let mut changes = self.inner.progress.subscribe();
            let meta = self.meta();
            if self.is_retired() {
                return Err(RangeError::Stale);
            }
            if meta.state != BuildState::Building {
                return Ok(meta);
            }
            tokio::select! {
                _ = changes.changed() => {}
                () = cancel.cancelled() => return Err(RangeError::Cancelled),
            }
        }
    }

    pub async fn read_range(
        &self,
        range: Range<u64>,
        cancel: &Cancel,
    ) -> Result<RangeData, RangeError> {
        loop {
            let mut changes = self.inner.progress.subscribe();
            if self.is_retired() {
                return Err(RangeError::Stale);
            }
            let (build, lines) = {
                let state = self.inner.state.lock().unwrap();
                (state.build, state.lines)
            };
            match build {
                BuildState::Cancelled => return Err(RangeError::Cancelled),
                BuildState::Complete => break,
                BuildState::Building if lines >= range.end => break,
                BuildState::Building => {}
            }
            tokio::select! {
                _ = changes.changed() => {}
                () = cancel.cancelled() => return Err(RangeError::Cancelled),
            }
        }
        let (kind, build, total, segments, tail, flushed) = {
            let state = self.inner.state.lock().unwrap();
            (
                state.kind.clone(),
                state.build,
                state.lines,
                state.segments.clone(),
                state.tail.clone(),
                state.flushed_lines,
            )
        };
        if build == BuildState::Complete
            && let Some(kind) = kind
            && kind != DocumentKind::Text
        {
            return Err(RangeError::NotReadable(kind));
        }
        if range.start > total {
            return Err(RangeError::OutOfRange {
                requested: range.start,
                total,
            });
        }
        let end = range.end.min(total);
        let mut lines = Vec::with_capacity(end.saturating_sub(range.start) as usize);
        let mut cursor = range.start;
        while cursor < end {
            if cursor >= flushed {
                let from = (cursor - flushed) as usize;
                let to = (end - flushed) as usize;
                lines.extend_from_slice(&tail[from..to]);
                break;
            }
            let index = segments.partition_point(|s| s.start + u64::from(s.lines) <= cursor);
            let meta = segments[index];
            let decoded = self.inner.segment(index as u32, meta, cancel).await?;
            let from = (cursor - meta.start) as usize;
            let to = ((end - meta.start) as usize).min(decoded.len());
            lines.extend_from_slice(&decoded[from..to]);
            cursor = meta.start + to as u64;
        }
        Ok(RangeData {
            start: range.start,
            lines,
            total_lines: (build == BuildState::Complete).then_some(total),
        })
    }
}

#[derive(Clone)]
pub struct DiffReader {
    git: Git,
    cache: DiffCache,
    documents: Arc<Mutex<HashMap<DiffKey, DiffDocument>>>,
}

impl DiffReader {
    pub fn new(git: Git, cache: DiffCache) -> Self {
        Self {
            git,
            cache,
            documents: Arc::default(),
        }
    }

    pub fn cache(&self) -> &DiffCache {
        &self.cache
    }

    pub fn open_document(
        &self,
        info: &SessionInfo,
        generation: Generation,
        side: DiffSide,
        entry: &ChangeEntry,
    ) -> Result<DiffDocument, DiffError> {
        let conflict = entry.kind == EntryKind::Unmerged;
        let visible = match side {
            DiffSide::Unstaged => entry.is_unstaged(),
            DiffSide::Staged => entry.is_staged(),
            DiffSide::Commit => false,
        };
        if !visible && !conflict {
            return Err(DiffError::NotInThisView);
        }
        self.open_inner(info, generation, side, entry, None)
    }

    pub fn open_commit_document(
        &self,
        info: &SessionInfo,
        generation: Generation,
        range: &CommitRange,
        entry: &ChangeEntry,
    ) -> Result<DiffDocument, DiffError> {
        self.open_inner(
            info,
            generation,
            DiffSide::Commit,
            entry,
            Some(range.clone()),
        )
    }

    fn open_inner(
        &self,
        info: &SessionInfo,
        generation: Generation,
        side: DiffSide,
        entry: &ChangeEntry,
        range: Option<CommitRange>,
    ) -> Result<DiffDocument, DiffError> {
        let worktree = info.worktree.as_ref().ok_or(DiffError::NoWorktree)?;
        let conflict = entry.kind == EntryKind::Unmerged;
        for protected in [worktree, &info.git_dir, &info.common_dir] {
            if let Ok(protected) = std::fs::canonicalize(protected)
                && self.cache.root().starts_with(&protected)
            {
                return Err(DiffError::CacheInsideRepository);
            }
        }
        let key = DiffKey {
            repo: info.repo.clone(),
            session: info.id,
            generation,
            side,
            file: FileRef {
                path: entry.path.clone(),
                original_path: entry.original_path.clone(),
            },
            range: range.clone(),
        };
        let mut documents = self.documents.lock().unwrap();
        if let Some(existing) = documents.get(&key)
            && !existing.is_retired()
            && existing.meta().state != BuildState::Cancelled
        {
            return Ok(existing.clone());
        }
        let (progress, _) = watch::channel(0);
        let inner = Arc::new(DocInner {
            key: key.clone(),
            id: self.cache.new_document(),
            git: self.git.clone(),
            cache: self.cache.clone(),
            request: Request {
                dir: worktree.clone(),
                side,
                path: entry.path.clone(),
                original: entry.original_path.clone(),
                untracked: entry.kind == EntryKind::Untracked,
                conflict,
                range,
            },
            cancel: Cancel::new(),
            retired: AtomicBool::new(false),
            state: Mutex::new(DocState {
                build: BuildState::Building,
                kind: None,
                lines: 0,
                added: 0,
                removed: 0,
                flushed_lines: 0,
                hunks: Vec::new(),
                type_changed: false,
                splitter: SplitIndexBuilder::new(),
                split: None,
                segments: Vec::new(),
                tail: Vec::new(),
                tail_bytes: 0,
            }),
            progress,
        });
        tokio::spawn(inner.clone().build());
        let document = DiffDocument { inner };
        documents.insert(key, document.clone());
        Ok(document)
    }

    fn retire_where(&self, retire: impl Fn(&DiffKey) -> bool) {
        let mut documents = self.documents.lock().unwrap();
        documents.retain(|key, document| {
            if !retire(key) {
                return true;
            }
            document.inner.retired.store(true, Ordering::SeqCst);
            document.inner.cancel.cancel();
            self.cache.remove_document(document.inner.id);
            document.inner.notify();
            false
        });
    }

    pub fn close_session(&self, session: SessionId) {
        self.retire_where(|key| key.session == session);
    }

    pub fn retire_before(&self, session: SessionId, generation: Generation) {
        self.retire_where(|key| key.session == session && key.generation < generation);
    }
}
