use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gitpoulpe_core::{
    BuildState, ChangeEntry, DiffLine, DiffSide, DocumentKind, DocumentMeta, Generation,
    StatusSnapshot,
};
use rune::prefs::Prefs;
use rune::{TextEdit, Tx, Waker};

use crate::engine::{Command, Engine, Event, WakerSlot};
use crate::theme::Palette;

pub const CHUNK_LINES: u64 = 256;
pub const LIST_WIDTH_DEFAULT: f32 = 372.0;
pub const LIST_WIDTH_MIN: f32 = 260.0;
pub const LIST_WIDTH_MAX: f32 = 640.0;
const LIST_WIDTH_KEY: &str = "list_width";
pub const READING_LINE_PERCENT: usize = 35;
const PREFETCH_BEFORE: usize = 2;
const PREFETCH_AFTER: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Unstaged,
    Staged,
}

impl ViewKind {
    pub fn side(self) -> DiffSide {
        match self {
            Self::Unstaged => DiffSide::Unstaged,
            Self::Staged => DiffSide::Staged,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Opening(PathBuf),
    Failed { path: PathBuf, message: String },
    Ready,
}

pub struct Repo {
    pub generation: Generation,
    pub snapshot: Arc<StatusSnapshot>,
    pub name: String,
}

#[derive(Default)]
pub struct DocView {
    pub meta: Option<DocumentMeta>,
    pub chunks: HashMap<u64, Vec<DiffLine>>,
    pub requested: HashSet<u64>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Layout {
    pub offsets: Vec<u64>,
    pub total: u64,
}

pub struct State {
    pub dark: bool,
    pub palette: Palette,
    pub screen: Screen,
    pub repo: Option<Repo>,
    pub view: ViewKind,
    pub filter: TextEdit,
    pub files: Vec<usize>,
    pub docs: HashMap<(ViewKind, usize), DocView>,
    pub layout: Layout,
    pub layout_version: u64,
    pub current: Option<usize>,
    pub pinned: Option<usize>,
    follow: Option<usize>,
    pub list_width: f32,
    prefs: Prefs,
    engine: Option<Engine>,
    events: Receiver<Event>,
    waker: WakerSlot,
    request: u64,
    layout_dirty: bool,
    initial_path: Option<PathBuf>,
    event_sender: Sender<Event>,
}

pub fn path_from_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .map(PathBuf::from)
        .find(|p| !p.to_string_lossy().starts_with('-') && p.is_dir())
}

fn initial_path() -> Option<PathBuf> {
    path_from_args(std::env::args_os())
}

impl Default for State {
    fn default() -> Self {
        Self::new(initial_path(), Prefs::open("gitpoulpe"))
    }
}

impl State {
    pub fn new(initial_path: Option<PathBuf>, prefs: Prefs) -> Self {
        let (event_sender, events) = channel();
        let dark = true;
        let list_width = prefs
            .get::<f32>(LIST_WIDTH_KEY)
            .map_or(LIST_WIDTH_DEFAULT, |w| {
                w.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX)
            });
        Self {
            dark,
            palette: Palette::of(dark),
            screen: Screen::Welcome,
            repo: None,
            view: ViewKind::Unstaged,
            filter: TextEdit::new(""),
            files: Vec::new(),
            docs: HashMap::new(),
            layout: Layout::default(),
            layout_version: 0,
            current: None,
            pinned: None,
            follow: None,
            list_width,
            prefs,
            engine: None,
            events,
            waker: Arc::new(Mutex::new(None)),
            request: 0,
            layout_dirty: false,
            initial_path,
            event_sender,
        }
    }

    pub fn setup(&mut self, waker: Waker) {
        *self.waker.lock().unwrap() = Some(waker);
        if self.engine.is_none() {
            self.engine = Engine::start(self.event_sender.clone(), self.waker.clone()).ok();
        }
        if let Some(path) = self.initial_path.take() {
            self.open_path(path);
        }
    }

    pub fn resize_list(&mut self, width: f32) {
        self.list_width = if width.is_finite() {
            width.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX)
        } else {
            LIST_WIDTH_DEFAULT
        };
    }

    pub fn save_list_width(&mut self) {
        let _ = self.prefs.set(LIST_WIDTH_KEY, &self.list_width);
    }

    pub fn jump_to(&mut self, position: usize) {
        self.current = Some(position);
        self.pinned = Some(position);
        self.follow = None;
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        self.palette = Palette::of(dark);
    }

    pub fn open_path(&mut self, path: PathBuf) {
        if self.engine.is_none() {
            self.engine = Engine::start(self.event_sender.clone(), self.waker.clone()).ok();
        }
        self.request += 1;
        self.screen = Screen::Opening(path.clone());
        if let Some(engine) = &self.engine {
            engine.send(Command::Open {
                request: self.request,
                path,
            });
        }
    }

    pub fn entries(&self) -> &[ChangeEntry] {
        self.repo
            .as_ref()
            .map_or(&[], |repo| &repo.snapshot.entries)
    }

    pub fn entry_at(&self, position: usize) -> Option<(usize, &ChangeEntry)> {
        let index = *self.files.get(position)?;
        self.entries().get(index).map(|entry| (index, entry))
    }

    pub fn count(&self, view: ViewKind) -> usize {
        self.entries()
            .iter()
            .filter(|e| Self::in_view(e, view))
            .count()
    }

    fn in_view(entry: &ChangeEntry, view: ViewKind) -> bool {
        match view {
            ViewKind::Unstaged => entry.is_unstaged() || entry.is_conflict(),
            ViewKind::Staged => entry.is_staged(),
        }
    }

    pub fn set_view(&mut self, view: ViewKind) {
        if self.view != view {
            self.view = view;
            self.current = None;
            self.pinned = None;
            self.refilter();
        }
    }

    pub fn refilter(&mut self) {
        let needle = self.filter.text.trim().to_lowercase();
        let view = self.view;
        self.files = self
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| Self::in_view(e, view))
            .filter(|(_, e)| {
                needle.is_empty() || e.path.to_string_lossy().to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        self.layout_dirty = true;
        self.relayout();
    }

    pub fn doc(&self, index: usize) -> Option<&DocView> {
        self.docs.get(&(self.view, index))
    }

    pub fn body_rows(&self, index: usize) -> u64 {
        let Some(meta) = self.doc(index).and_then(|d| d.meta.as_ref()) else {
            return 1;
        };
        match (&meta.kind, meta.state) {
            (Some(DocumentKind::Text), _) | (None, BuildState::Building) => meta.lines.max(1),
            _ => 1,
        }
    }

    fn relayout(&mut self) {
        let mut offsets = Vec::with_capacity(self.files.len() + 1);
        let mut total = 0;
        for &index in &self.files {
            offsets.push(total);
            total += 1 + self.body_rows(index);
        }
        self.layout = Layout { offsets, total };
        self.layout_version += 1;
        self.layout_dirty = false;
    }

    pub fn locate(&self, row: u64) -> Option<(usize, u64)> {
        if row >= self.layout.total {
            return None;
        }
        let position = self
            .layout
            .offsets
            .partition_point(|&start| start <= row)
            .checked_sub(1)?;
        Some((position, row - self.layout.offsets[position]))
    }

    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            changed |= self.apply(event);
        }
        if self.layout_dirty {
            self.relayout();
            changed = true;
        }
        changed
    }

    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Opened {
                request,
                info,
                snapshot,
            } => {
                if request != self.request {
                    return false;
                }
                let name = info
                    .worktree
                    .as_deref()
                    .and_then(Path::file_name)
                    .map_or_else(|| "dépôt".to_owned(), |n| n.to_string_lossy().into_owned());
                self.repo = Some(Repo {
                    generation: snapshot.generation,
                    snapshot: Arc::new(*snapshot),
                    name,
                });
                self.docs.clear();
                self.current = None;
                self.pinned = None;
                self.screen = Screen::Ready;
                self.refilter();
                true
            }
            Event::OpenFailed { request, message } => {
                if request != 0 && request != self.request {
                    return false;
                }
                let path = match &self.screen {
                    Screen::Opening(path) => path.clone(),
                    _ => PathBuf::new(),
                };
                self.screen = Screen::Failed { path, message };
                true
            }
            Event::Document {
                generation,
                side,
                index,
                meta,
            } => {
                let Some(view) = self.matching_view(generation, side) else {
                    return false;
                };
                let doc = self.docs.entry((view, index)).or_default();
                let counts_changed = doc
                    .meta
                    .as_ref()
                    .map(|m| (m.lines, m.kind.clone(), m.state))
                    != Some((meta.lines, meta.kind.clone(), meta.state));
                doc.meta = Some(*meta);
                if counts_changed && view == self.view {
                    self.layout_dirty = true;
                }
                counts_changed
            }
            Event::Lines {
                generation,
                side,
                index,
                start,
                lines,
            } => {
                let Some(view) = self.matching_view(generation, side) else {
                    return false;
                };
                self.docs
                    .entry((view, index))
                    .or_default()
                    .chunks
                    .insert(start / CHUNK_LINES, lines);
                true
            }
            Event::ReadFailed {
                generation,
                side,
                index,
                message,
            } => {
                let Some(view) = self.matching_view(generation, side) else {
                    return false;
                };
                self.docs.entry((view, index)).or_default().error = Some(message);
                true
            }
        }
    }

    fn matching_view(&self, generation: Generation, side: DiffSide) -> Option<ViewKind> {
        let repo = self.repo.as_ref()?;
        if repo.generation != generation {
            return None;
        }
        Some(match side {
            DiffSide::Unstaged => ViewKind::Unstaged,
            DiffSide::Staged => ViewKind::Staged,
        })
    }

    fn request_chunk(&mut self, position: usize, chunk: u64) {
        let Some((index, entry)) = self.entry_at(position).map(|(i, e)| (i, e.clone())) else {
            return;
        };
        let Some(generation) = self.repo.as_ref().map(|r| r.generation) else {
            return;
        };
        let doc = self.docs.entry((self.view, index)).or_default();
        if doc.chunks.contains_key(&chunk) || !doc.requested.insert(chunk) {
            return;
        }
        if let Some(engine) = &self.engine {
            engine.send(Command::Read {
                generation,
                side: self.view.side(),
                index,
                entry: Box::new(entry),
                range: chunk * CHUNK_LINES..(chunk + 1) * CHUNK_LINES,
            });
        }
    }

    fn request_range(&mut self, rows: &Range<usize>) -> Option<(usize, usize)> {
        if self.files.is_empty() || rows.is_empty() {
            return None;
        }
        let (Some((first, first_local)), Some((last, last_local))) = (
            self.locate(rows.start as u64),
            self.locate(rows.end.saturating_sub(1) as u64),
        ) else {
            return None;
        };
        let from = first.saturating_sub(PREFETCH_BEFORE);
        let to = (last + PREFETCH_AFTER).min(self.files.len() - 1);
        for position in from..=to {
            self.request_chunk(position, 0);
        }
        for position in first..=last {
            let start = if position == first {
                first_local.saturating_sub(1)
            } else {
                0
            };
            let end = if position == last {
                last_local
            } else {
                self.body_rows(self.files[position])
            };
            for chunk in (start / CHUNK_LINES)..=(end / CHUNK_LINES) {
                self.request_chunk(position, chunk);
            }
        }
        Some((first, last))
    }

    pub fn on_visible(&mut self, rows: Range<usize>) -> bool {
        let Some((first, last)) = self.request_range(&rows) else {
            return false;
        };
        let at_end = rows.end as u64 >= self.layout.total;
        let reading_row = rows.start + rows.len() * READING_LINE_PERCENT / 100;
        let reading = self
            .locate(reading_row as u64)
            .map_or(first, |(position, _)| position);
        let by_scroll = if at_end {
            last
        } else if rows.start == 0 {
            first
        } else {
            reading
        };
        let pinned = self
            .pinned
            .filter(|&p| (first..=last).contains(&p) && (p == first || at_end));
        self.pinned = pinned;
        let current = pinned.unwrap_or(by_scroll);
        let changed = self.current != Some(current);
        self.current = Some(current);
        if changed {
            self.follow = self.entry_at(current).map(|(index, _)| index);
        }
        changed
    }

    pub fn apply_follow(&mut self, tx: &mut Tx) {
        if let Some(index) = self.follow.take() {
            tx.scroll_into_view_in("list-rows", format!("f{index}"));
        }
    }

    pub fn line(&self, index: usize, line: u64) -> Option<&DiffLine> {
        self.doc(index)?
            .chunks
            .get(&(line / CHUNK_LINES))?
            .get((line % CHUNK_LINES) as usize)
    }

    pub fn settle(&mut self, files: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        self.request_range(&(0..self.layout.total.min(files as u64 * 2) as usize));
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            self.drain();
            let ready = self.files.iter().take(files).all(|&index| {
                self.doc(index).is_some_and(|d| {
                    d.meta
                        .as_ref()
                        .is_some_and(|m| m.state != BuildState::Building)
                        && (d.chunks.contains_key(&0)
                            || d.error.is_some()
                            || d.meta
                                .as_ref()
                                .is_some_and(|m| m.kind != Some(DocumentKind::Text)))
                })
            });
            if ready {
                return;
            }
        }
    }

    pub fn open_blocking(&mut self, path: PathBuf, timeout: Duration) {
        self.open_path(path);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline
            && !matches!(self.screen, Screen::Ready | Screen::Failed { .. })
        {
            std::thread::sleep(Duration::from_millis(20));
            self.drain();
        }
    }
}
