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

pub enum Command {
    Open {
        request: u64,
        path: PathBuf,
    },
    Read {
        generation: Generation,
        side: DiffSide,
        index: usize,
        entry: Box<ChangeEntry>,
        range: Range<u64>,
    },
}

pub enum Event {
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
