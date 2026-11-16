use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use gitpoulpe_core::{
    ChangeEntry, DiffLine, DiffSide, DocumentMeta, Generation, OpenError, RangeError, SessionInfo,
    StatusSnapshot,
};
use gitpoulpe_git::{CacheConfig, Cancel, DiffCache, DiffReader, Git};
use rune::Waker;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const MAX_COPY_BYTES: usize = 32 * 1024 * 1024;
const COPY_SPAN_LINES: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyRequest {
    pub request: u64,
    pub generation: Generation,
    pub side: DiffSide,
    pub files: Vec<(usize, ChangeEntry)>,
    pub from_row: u64,
    pub from_offset: usize,
    pub to_row: u64,
    pub to_offset: usize,
    pub split_left: Option<bool>,
    pub max_bytes: usize,
}

pub enum Command {
    Open {
        request: u64,
        path: PathBuf,
    },
    Copy(Box<CopyRequest>),
    Read {
        generation: Generation,
        side: DiffSide,
        index: usize,
        entry: Box<ChangeEntry>,
        range: Range<u64>,
    },
}

pub enum Event {
    Copied {
        request: u64,
        text: String,
        lines: u64,
        truncated: bool,
    },
    Opened {
        request: u64,
        info: Box<SessionInfo>,
        snapshot: Box<StatusSnapshot>,
    },
    OpenFailed {
        request: u64,
        message: String,
    },
    Document {
        generation: Generation,
        side: DiffSide,
        index: usize,
        meta: Box<DocumentMeta>,
    },
    Lines {
        generation: Generation,
        side: DiffSide,
        index: usize,
        start: u64,
        lines: Vec<DiffLine>,
    },
    ReadFailed {
        generation: Generation,
        side: DiffSide,
        index: usize,
        message: String,
    },
}

pub type WakerSlot = Arc<Mutex<Option<Waker>>>;

#[derive(Clone)]
pub struct Engine {
    commands: UnboundedSender<Command>,
}

impl Engine {
    pub fn start(events: Sender<Event>, waker: WakerSlot) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("gitpoulpe-engine")
            .enable_all()
            .build()?;
        let (commands, receiver) = unbounded_channel();
        std::thread::Builder::new()
            .name("gitpoulpe-engine-main".to_owned())
            .spawn(move || {
                runtime.block_on(run(receiver, events, waker));
            })?;
        Ok(Self { commands })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

struct Session {
    info: SessionInfo,
    generation: Generation,
}

struct Shared {
    git: Git,
    reader: DiffReader,
    events: Sender<Event>,
    waker: WakerSlot,
}

impl Shared {
    fn emit(&self, event: Event) {
        if self.events.send(event).is_ok()
            && let Some(wake) = self.waker.lock().unwrap().as_ref()
        {
            wake();
        }
    }
}

fn open_cache() -> std::io::Result<DiffCache> {
    DiffCache::open(CacheConfig::new(DiffCache::default_dir())).or_else(|first| {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|dir| std::path::PathBuf::from(dir).join("gitpoulpe"));
        match runtime {
            Some(dir) => DiffCache::open(CacheConfig::new(dir)),
            None => Err(first),
        }
    })
}

async fn run(mut receiver: UnboundedReceiver<Command>, events: Sender<Event>, waker: WakerSlot) {
    let git = Git::new();
    let cache = match open_cache() {
        Ok(cache) => cache,
        Err(error) => {
            let _ = events.send(Event::OpenFailed {
                request: 0,
                message: format!("Cache indisponible : {error}"),
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
            return;
        }
    };
    let shared = Arc::new(Shared {
        reader: DiffReader::new(git.clone(), cache),
        git,
        events,
        waker,
    });
    let mut session: Option<Arc<Session>> = None;
    let watched = Arc::new(std::sync::Mutex::new(HashSet::new()));
    while let Some(command) = receiver.recv().await {
        match command {
            Command::Open { request, path } => {
                if let Some(previous) = session.take() {
                    shared.reader.close_session(previous.info.id);
                    watched.lock().unwrap().clear();
                }
                match open(&shared, &path).await {
                    Ok((info, snapshot)) => {
                        session = Some(Arc::new(Session {
                            info: info.clone(),
                            generation: snapshot.generation,
                        }));
                        shared.emit(Event::Opened {
                            request,
                            info: Box::new(info),
                            snapshot: Box::new(snapshot),
                        });
                    }
                    Err(message) => shared.emit(Event::OpenFailed { request, message }),
                }
            }
            Command::Copy(request) => {
                let Some(current) = session
                    .clone()
                    .filter(|s| s.generation == request.generation)
                else {
                    continue;
                };
                tokio::spawn(copy(shared.clone(), current, *request));
            }
            Command::Read {
                generation,
                side,
                index,
                entry,
                range,
            } => {
                let Some(current) = session.clone().filter(|s| s.generation == generation) else {
                    continue;
                };
                tokio::spawn(read(
                    shared.clone(),
                    current,
                    watched.clone(),
                    side,
                    index,
                    *entry,
                    range,
                ));
            }
        }
    }
}

async fn open(
    shared: &Shared,
    path: &std::path::Path,
) -> Result<(SessionInfo, StatusSnapshot), String> {
    let cancel = Cancel::new();
    let info = shared
        .git
        .open(path, &cancel)
        .await
        .map_err(describe_open_error)?;
    let snapshot = shared
        .git
        .read_status(&info, Generation::default().next(), &cancel)
        .await
        .map_err(|e| e.to_string())?;
    Ok((info, snapshot))
}

fn describe_open_error(error: OpenError) -> String {
    match error {
        OpenError::NotFound => "Ce dossier n'existe pas.".to_owned(),
        OpenError::NotADirectory => "Ce chemin n'est pas un dossier.".to_owned(),
        OpenError::PermissionDenied => "L'accès à ce dossier est refusé.".to_owned(),
        OpenError::NotARepository => "Ce dossier n'est pas un dépôt Git.".to_owned(),
        OpenError::GitUnavailable(d) => format!("Git est introuvable : {d}"),
        OpenError::GitFailed(d) => format!("Git a échoué : {d}"),
        OpenError::Cancelled => "Ouverture annulée.".to_owned(),
    }
}

type Watched = Arc<std::sync::Mutex<HashSet<(DiffSide, usize)>>>;

async fn read(
    shared: Arc<Shared>,
    session: Arc<Session>,
    watched: Watched,
    side: DiffSide,
    index: usize,
    entry: ChangeEntry,
    range: Range<u64>,
) {
    let generation = session.generation;
    let document = match shared
        .reader
        .open_document(&session.info, generation, side, &entry)
    {
        Ok(document) => document,
        Err(error) => {
            return shared.emit(Event::ReadFailed {
                generation,
                side,
                index,
                message: error.to_string(),
            });
        }
    };
    let cancel = Cancel::new();
    shared.emit(Event::Document {
        generation,
        side,
        index,
        meta: Box::new(document.meta()),
    });
    match document.read_range(range.clone(), &cancel).await {
        Ok(data) => shared.emit(Event::Lines {
            generation,
            side,
            index,
            start: data.start,
            lines: data.lines,
        }),
        Err(RangeError::NotReadable(_)) => {}
        Err(RangeError::Cancelled) => return,
        Err(error) => {
            return shared.emit(Event::ReadFailed {
                generation,
                side,
                index,
                message: error.to_string(),
            });
        }
    }
    shared.emit(Event::Document {
        generation,
        side,
        index,
        meta: Box::new(document.meta()),
    });
    let first_watcher = watched.lock().unwrap().insert((side, index));
    if first_watcher && let Ok(meta) = document.wait_finished(&cancel).await {
        shared.emit(Event::Document {
            generation,
            side,
            index,
            meta: Box::new(meta),
        });
    }
}

fn clip_to_offsets(line: &str, from: Option<usize>, to: Option<usize>) -> &str {
    let mut start = from.unwrap_or(0).min(line.len());
    let mut end = to.unwrap_or(line.len()).clamp(start, line.len());
    while !line.is_char_boundary(start) {
        start += 1;
    }
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[start..end.max(start)]
}

async fn copy(shared: Arc<Shared>, session: Arc<Session>, request: CopyRequest) {
    let cancel = Cancel::new();
    let mut text = String::new();
    let mut lines_copied = 0u64;
    let mut truncated = false;
    let last_file = request.files.len().saturating_sub(1);
    'files: for (position, (_, entry)) in request.files.iter().enumerate() {
        let Ok(document) =
            shared
                .reader
                .open_document(&session.info, request.generation, request.side, entry)
        else {
            continue;
        };
        let Ok(meta) = document.wait_finished(&cancel).await else {
            continue;
        };
        if meta.kind != Some(gitpoulpe_core::DocumentKind::Text) {
            continue;
        }
        let split = match request.split_left {
            Some(_) => meta.split.clone(),
            None => None,
        };
        let total_rows = split.as_ref().map_or(meta.lines, |s| s.total_rows());
        if total_rows == 0 {
            continue;
        }
        let first_row = if position == 0 { request.from_row } else { 0 };
        let last_row = if position == last_file {
            request.to_row.min(total_rows - 1)
        } else {
            total_rows - 1
        };
        let line_of = |row: u64| match (&split, request.split_left) {
            (Some(split), Some(left)) => split
                .cells(row)
                .and_then(|c| if left { c.left } else { c.right }),
            _ => Some(row),
        };
        let first_line = line_of(first_row);
        let last_line = line_of(last_row);
        let mut wanted: Vec<u64> = Vec::new();
        for row in first_row..=last_row {
            if let Some(line) = line_of(row) {
                wanted.push(line);
            }
        }
        wanted.dedup();
        let mut cursor = 0;
        while cursor < wanted.len() {
            let start = wanted[cursor];
            let mut end = start + 1;
            let mut taken = 1;
            while cursor + taken < wanted.len()
                && wanted[cursor + taken] == end
                && end - start < COPY_SPAN_LINES
            {
                end += 1;
                taken += 1;
            }
            cursor += taken;
            let Ok(data) = document.read_range(start..end, &cancel).await else {
                continue 'files;
            };
            for (offset, line) in data.lines.iter().enumerate() {
                use gitpoulpe_core::DiffLineKind::{HunkHeader, NoNewlineAtEof};
                if matches!(line.kind, HunkHeader | NoNewlineAtEof) {
                    continue;
                }
                let id = start + offset as u64;
                let content = String::from_utf8_lossy(&line.text);
                let from = (position == 0 && Some(id) == first_line).then_some(request.from_offset);
                let to =
                    (position == last_file && Some(id) == last_line).then_some(request.to_offset);
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(clip_to_offsets(&content, from, to));
                lines_copied += 1;
                if text.len() > request.max_bytes {
                    truncated = true;
                    break 'files;
                }
            }
        }
    }
    shared.emit(Event::Copied {
        request: request.request,
        text,
        lines: lines_copied,
        truncated,
    });
}
