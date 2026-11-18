use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::history_state::{HistoryState, SearchState};
use crate::profiles::{IdentityShown, PROFILES_KEY, ProfileForm, SigningKind};
use gitpoulpe_core::{
    AmendInfo, AmendRequest, CommitDetail, CommitFailureKind, CommitOutcome, CommitRefusal,
    CommitRequest, JournalEntry, Profile, ProfileBook, push_journal,
};
use gitpoulpe_core::{
    BuildState, ChangeEntry, DiffLine, DiffSide, DocumentKind, DocumentMeta, Generation, HunkLines,
    HunkSpec, LineSelection, Refusal, SplitIndex, StageOutcome, StageTarget, StatusSnapshot,
    WriteError, WriteJob, display_label,
};
use gitpoulpe_git::{Git, SigningCheck};
use rune::prefs::Prefs;
use rune::{Live, TextEdit, Tx, Waker};

use crate::engine::{Command, CopyRequest, Engine, Event, WakerSlot};
use crate::theme::Palette;

pub const CHUNK_LINES: u64 = 256;
pub const LIST_WIDTH_DEFAULT: f32 = 372.0;
pub const LIST_WIDTH_MIN: f32 = 260.0;
pub const LIST_WIDTH_MAX: f32 = 640.0;
pub const LIST_SHARE_OF_WINDOW: f32 = 0.45;
const LIST_WIDTH_KEY: &str = "list_width";
const MODE_KEY: &str = "diff_mode";
pub const READING_LINE_PERCENT: usize = 35;
const STICK_GRACE_CALLS: u8 = 2;
const REFRESHING_NOTICE: &str = "Actualisation…";
const ANCHOR_FRAMES: u16 = 90;
const ANCHOR_STABLE_FRAMES: u8 = 3;
const FOCUS_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(1);
pub const MAX_LOADED_LINES: usize = 30_000;
const KEEP_FILES: usize = 40;
const KEEP_CHUNKS: u64 = 6;
const PREFETCH_BEFORE: usize = 2;
const PREFETCH_AFTER: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Unstaged,
    Staged,
    Commit,
}

impl ViewKind {
    pub fn side(self) -> DiffSide {
        match self {
            Self::Unstaged => DiffSide::Unstaged,
            Self::Staged => DiffSide::Staged,
            Self::Commit => DiffSide::Commit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedCommit {
    pub detail: CommitDetail,
    pub entries: Vec<ChangeEntry>,
    pub base_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinePick {
    pub index: usize,
    pub anchor: u64,
    pub spans: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchStatus {
    Unknown,
    Active,
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingAnchor {
    pub path: PathBuf,
    pub line: Option<u64>,
}

struct PendingAnchor {
    anchor: ReadingAnchor,
    stable_frames: u8,
    frames_left: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionAnchor {
    pub index: usize,
    pub row: u64,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DiffMode {
    #[default]
    Unified,
    Split,
}

impl DiffMode {
    fn pref(self) -> &'static str {
        match self {
            Self::Unified => "unified",
            Self::Split => "split",
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
    pub mode: DiffMode,
    pub filter: TextEdit,
    pub files: Vec<usize>,
    pub docs: HashMap<(ViewKind, usize), DocView>,
    pub layout: Layout,
    pub layout_version: u64,
    pub current: Option<usize>,
    pub pinned: Option<usize>,
    pub visible: Range<usize>,
    pub notice: Option<String>,
    pub marked: HashSet<PathBuf>,
    pub marks_version: u32,
    pub line_pick: Option<LinePick>,
    pub history_open: bool,
    pub inspected: Option<InspectedCommit>,
    inspect_request: u64,
    pub return_to_history: bool,
    pub history: HistoryState,
    history_request: u64,
    pub search_text: TextEdit,
    pub search: SearchState,
    pub pending_reveal: Option<String>,
    pub book: ProfileBook,
    pub commit_message: TextEdit,
    pub commit_report: Option<String>,
    commit_request: u64,
    pub amend_mode: bool,
    pub amend_info: Option<AmendInfo>,
    pub amend_confirming: bool,
    amend_info_request: u64,
    submitted_amend: bool,
    pub identity_shown: Option<IdentityShown>,
    pub profiles_open: bool,
    pub ignored: crate::ignored::IgnoredState,
    pub branches: crate::branches::BranchesState,
    pub tags: crate::tags::TagsState,
    pub remotes: crate::remotes::RemotesState,
    pub init: crate::initialize::InitState,
    pub conflicts: crate::conflicts::ConflictsState,
    pub stashes: crate::stashes::StashesState,
    pub resetting: crate::resetting::ResetSlot,
    pub sequencing: crate::sequencing::SequenceSlot,
    pub reflog: crate::reflogs::ReflogState,
    pub publishing: crate::publishing::PublishSlot,
    pub operation_kind: Option<gitpoulpe_core::OperationKind>,
    pub(crate) probe_request: u64,
    pub clone: crate::cloning::CloneState,
    pub engine_git: Option<Git>,
    pub form: ProfileForm,
    identity_request: u64,
    signing_request: u64,
    pub pick_version: u32,
    pub writing: bool,
    stage_request: u64,
    pub watch: WatchStatus,
    pub refresh_request: u64,
    last_refresh: Option<Instant>,
    positions: HashMap<ViewKind, ReadingAnchor>,
    pending_anchor: Option<PendingAnchor>,
    perf: Option<crate::perf::Perf>,
    pub max_loaded_lines: usize,
    visible_chunks: HashMap<usize, (u64, u64)>,
    copy_request: u64,
    pub copy_limit: usize,
    copied: Option<(String, u64, bool)>,
    pub stick_end: bool,
    stick_grace: u8,
    last_offset: f32,
    pub scroll_probe: Live<[f32; 4]>,
    pub scroll_x: Live<[f32; 2]>,
    pub window_width: f32,
    follow: Option<usize>,
    pub list_width: f32,
    prefs: Prefs,
    pub(crate) engine: Option<Engine>,
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
        let mode = match prefs.get::<String>(MODE_KEY).as_deref() {
            Some("split") => DiffMode::Split,
            _ => DiffMode::Unified,
        };
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
            mode,
            filter: TextEdit::new(""),
            files: Vec::new(),
            docs: HashMap::new(),
            layout: Layout::default(),
            layout_version: 0,
            current: None,
            pinned: None,
            visible: 0..0,
            notice: None,
            watch: WatchStatus::Unknown,
            refresh_request: 0,
            last_refresh: None,
            positions: HashMap::new(),
            pending_anchor: None,
            perf: crate::perf::Perf::from_env(),
            max_loaded_lines: MAX_LOADED_LINES,
            visible_chunks: HashMap::new(),
            copy_request: 0,
            marked: HashSet::new(),
            marks_version: 0,
            line_pick: None,
            history_open: false,
            inspected: None,
            inspect_request: 0,
            return_to_history: false,
            history: HistoryState::default(),
            history_request: 0,
            search_text: TextEdit::new(""),
            search: SearchState::default(),
            pending_reveal: None,
            book: prefs.get::<ProfileBook>(PROFILES_KEY).unwrap_or_default(),
            commit_message: TextEdit::new(""),
            commit_report: None,
            commit_request: 0,
            amend_mode: false,
            amend_info: None,
            amend_confirming: false,
            amend_info_request: 0,
            submitted_amend: false,
            identity_shown: None,
            profiles_open: false,
            ignored: crate::ignored::IgnoredState::default(),
            branches: crate::branches::BranchesState::default(),
            tags: crate::tags::TagsState::default(),
            remotes: crate::remotes::RemotesState::default(),
            init: crate::initialize::InitState::default(),
            conflicts: crate::conflicts::ConflictsState::default(),
            stashes: crate::stashes::StashesState::default(),
            resetting: crate::resetting::ResetSlot::default(),
            sequencing: crate::sequencing::SequenceSlot::default(),
            reflog: crate::reflogs::ReflogState::default(),
            publishing: crate::publishing::PublishSlot::default(),
            operation_kind: None,
            probe_request: 0,
            clone: crate::cloning::CloneState::default(),
            engine_git: None,
            form: ProfileForm::default(),
            identity_request: 0,
            signing_request: 0,
            pick_version: 0,
            writing: false,
            stage_request: 0,
            copy_limit: crate::engine::MAX_COPY_BYTES,
            copied: None,
            stick_end: false,
            stick_grace: 0,
            last_offset: 0.0,
            scroll_probe: Live::new([0.0; 4]),
            scroll_x: Live::new([0.0; 2]),
            window_width: 0.0,
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
            self.engine = Engine::start(
                self.event_sender.clone(),
                self.waker.clone(),
                self.engine_git.clone().unwrap_or_default(),
            )
            .ok();
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

    pub fn effective_list_width(&self) -> f32 {
        let window = self.window_width;
        if window > 0.0 {
            self.list_width
                .min((window * LIST_SHARE_OF_WINDOW).max(LIST_WIDTH_MIN))
        } else {
            self.list_width
        }
    }

    pub fn save_list_width(&mut self) {
        let _ = self.prefs.set(LIST_WIDTH_KEY, &self.list_width);
    }

    pub fn jump_to(&mut self, position: usize) {
        self.current = Some(position);
        self.pinned = Some(position);
        self.follow = None;
        self.stick_end = false;
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        self.palette = Palette::of(dark);
    }

    pub fn open_path(&mut self, path: PathBuf) {
        if self.engine.is_none() {
            self.engine = Engine::start(
                self.event_sender.clone(),
                self.waker.clone(),
                self.engine_git.clone().unwrap_or_default(),
            )
            .ok();
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
        if self.view == ViewKind::Commit
            && let Some(inspected) = &self.inspected
        {
            return &inspected.entries;
        }
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
            ViewKind::Commit => true,
        }
    }

    pub fn set_view(&mut self, view: ViewKind) {
        if self.view == ViewKind::Commit && view != ViewKind::Commit {
            self.leave_commit();
            if view == ViewKind::Commit {
                return;
            }
            self.return_to_history = false;
            self.history_open = false;
            if view != self.view {
                self.set_view(view);
            }
            return;
        }
        if self.view != view {
            if let Some(anchor) = self.capture_anchor() {
                self.positions.insert(self.view, anchor);
            }
            self.view = view;
            self.clear_pick();
            self.current = None;
            self.pinned = None;
            self.stick_end = false;
            self.refilter();
            if let Some(anchor) = self.positions.get(&view).cloned() {
                self.relocate(Some(anchor), &[], false);
            }
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

    pub fn split_index(&self, index: usize) -> Option<&Arc<SplitIndex>> {
        if self.mode != DiffMode::Split {
            return None;
        }
        self.doc(index)?.meta.as_ref()?.split.as_ref()
    }

    pub fn body_rows(&self, index: usize) -> u64 {
        let Some(meta) = self.doc(index).and_then(|d| d.meta.as_ref()) else {
            return 1;
        };
        match (&meta.kind, meta.state) {
            (Some(DocumentKind::Text), _) | (None, BuildState::Building) => {
                match self.split_index(index) {
                    Some(split) => split.total_rows().max(1),
                    None => meta.lines.max(1),
                }
            }
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

    fn watch_scroll(&mut self) {
        let offset = self.scroll_probe.get()[0];
        if self.stick_end {
            if self.stick_grace > 0 {
                self.stick_grace -= 1;
            } else if offset < self.last_offset - 1.0 {
                self.stick_end = false;
            }
        }
        self.last_offset = offset;
    }

    pub fn drain(&mut self) -> bool {
        self.watch_scroll();
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
                self.marked.clear();
                self.clear_pick();
                self.history_open = false;
                self.history = HistoryState::default();
                self.ignored = crate::ignored::IgnoredState::default();
                self.branches = crate::branches::BranchesState::default();
                self.tags = crate::tags::TagsState::default();
                self.remotes = crate::remotes::RemotesState::default();
                self.conflicts = crate::conflicts::ConflictsState::default();
                self.stashes = crate::stashes::StashesState::default();
                self.resetting = crate::resetting::ResetSlot::default();
                self.sequencing = crate::sequencing::SequenceSlot::default();
                self.reflog = crate::reflogs::ReflogState::default();
                self.publishing = crate::publishing::PublishSlot::default();
                self.operation_kind = None;
                self.writing = false;
                self.positions.clear();
                self.pending_anchor = None;
                self.current = None;
                self.pinned = None;
                self.stick_end = false;
                self.screen = Screen::Ready;
                self.refilter();
                self.request_identity();
                self.probe_operation();
                true
            }
            Event::Refreshed {
                request,
                snapshot,
                events_lost,
            } => self.apply_refresh(request, *snapshot, events_lost),
            Event::RefreshFailed { request, message } => {
                if request != self.refresh_request {
                    return false;
                }
                self.notice = Some(format!("Actualisation impossible : {message}"));
                true
            }
            Event::WatchState { available, reason } => {
                self.watch = if available {
                    WatchStatus::Active
                } else {
                    WatchStatus::Unavailable(reason)
                };
                true
            }
            Event::CommitInspected {
                request,
                generation,
                detail,
                base_index,
                entries,
            } => {
                if request != self.inspect_request {
                    return false;
                }
                if let Some(repo) = self.repo.as_mut() {
                    repo.generation = generation;
                }
                if self.view != ViewKind::Commit {
                    self.return_to_history = self.history_open;
                }
                self.history_open = false;
                self.inspected = Some(InspectedCommit {
                    detail: *detail,
                    entries,
                    base_index,
                });
                self.view = ViewKind::Commit;
                self.docs.clear();
                self.marked.clear();
                self.clear_pick();
                self.current = None;
                self.pinned = None;
                self.stick_end = false;
                self.notice = None;
                self.refilter();
                true
            }
            Event::CommitInspectFailed { request, message } => {
                if request != self.inspect_request {
                    return false;
                }
                self.notice = Some(format!("Impossible d'inspecter ce commit : {message}"));
                true
            }
            Event::HistoryPage {
                request,
                commits,
                done,
            } => {
                if request != self.history.request {
                    return false;
                }
                self.history.push_page(commits, done);
                true
            }
            Event::HistoryFailed {
                request,
                no_commits,
                message,
            } => {
                if request != self.history.request {
                    return false;
                }
                self.history.fail(no_commits, message);
                true
            }
            Event::SearchPage {
                request,
                commits,
                done,
            } => {
                if request != self.search.request {
                    return false;
                }
                self.search.push_page(commits, done);
                true
            }
            Event::SearchFailed { request, message } => {
                if request != self.search.request {
                    return false;
                }
                self.search.fail(message);
                true
            }
            Event::AmendInfo { request, info } => {
                if request != self.amend_info_request || !self.amend_mode {
                    return false;
                }
                if let Some(info) = &info
                    && self.commit_message.text.trim().is_empty()
                {
                    self.commit_message = TextEdit::new(&info.message);
                }
                self.amend_info = info;
                if self.amend_info.is_none() {
                    self.amend_mode = false;
                    self.notice = Some("Il n'y a aucun commit à amender.".to_owned());
                }
                true
            }
            Event::Committed { request, result } => {
                if request != self.commit_request {
                    return false;
                }
                self.writing = false;
                self.finish_commit(result);
                true
            }
            Event::InitPrepared { request, branch } => self.apply_init_prepared(request, branch),
            Event::InitDone {
                request,
                path,
                completed_existing,
            } => self.apply_init_done(request, path, completed_existing),
            Event::InitFailed { request, message } => self.apply_init_failed(request, message),
            Event::RemotesListed { request, result } => self.apply_remotes_listed(request, result),
            Event::RemoteDone { request, result } => self.apply_remote_done(request, result),
            Event::RemoteImpacted {
                request,
                name,
                result,
            } => self.apply_remote_impacted(request, name, result),
            Event::RemoteChecked {
                request,
                name,
                result,
            } => self.apply_remote_checked(request, name, result),
            Event::FetchProgress {
                request,
                phase,
                percent,
            } => self.apply_fetch_progress(request, phase, percent),
            Event::FetchDone { request, result } => self.apply_fetch_done(request, result),
            Event::CloneProgress {
                request,
                phase,
                percent,
            } => self.apply_clone_progress(request, phase, percent),
            Event::CloneDone { request, result } => self.apply_clone_done(request, result),
            Event::OperationProbed { request, kind } => self.apply_operation_probed(request, kind),
            Event::ConflictsLoaded { request, result } => {
                self.apply_conflicts_loaded(request, result)
            }
            Event::ConflictVersionsLoaded { request, result } => {
                self.apply_conflict_versions(request, result)
            }
            Event::ConflictFinished { request, result } => {
                self.apply_conflict_finished(request, result)
            }
            Event::MergePreviewed { request, result } => {
                self.apply_merge_previewed(request, result)
            }
            Event::MergeFinished { request, result } => self.apply_merge_finished(request, result),
            Event::StashesListed { request, result } => self.apply_stashes_listed(request, result),
            Event::StashDetailed {
                request,
                oid,
                result,
            } => self.apply_stash_detailed(request, &oid, result),
            Event::StashFinished { request, result } => self.apply_stash_finished(request, result),
            Event::PublishPrepared {
                request,
                remotes,
                preview,
            } => self.apply_publish_prepared(request, remotes, preview),
            Event::PushPreviewed { request, result } => self.apply_push_previewed(request, result),
            Event::PushProgress {
                request,
                phase,
                percent,
            } => self.apply_push_progress(request, phase, percent),
            Event::PushFinished { request, result } => self.apply_push_finished(request, result),
            Event::ReflogListed { request, result } => self.apply_reflog_listed(request, result),
            Event::ReflogInspected { request, result } => {
                self.apply_reflog_inspected(request, result)
            }
            Event::ReflogRecovered { request, result } => {
                self.apply_reflog_recovered(request, result)
            }
            Event::ResetPreviewed { request, result } => {
                self.apply_reset_previewed(request, result)
            }
            Event::ResetFinished { request, result } => self.apply_reset_finished(request, result),
            Event::SequencePreviewed { request, result } => {
                self.apply_sequence_previewed(request, result)
            }
            Event::SequenceFinished { request, result } => {
                self.apply_sequence_finished(request, result)
            }
            Event::SshScanned { request, result } => self.apply_ssh_scanned(request, result),
            Event::SshTrusted { request, result } => self.apply_ssh_trusted(request, result),
            Event::TagsListed { request, result } => self.apply_tags_listed(request, result),
            Event::TagDone { request, result } => self.apply_tag_done(request, result),
            Event::BranchesListed { request, result } => {
                self.apply_branches_listed(request, result)
            }
            Event::BranchDone { request, result } => self.apply_branch_done(request, result),
            Event::BranchNeedsConfirmation {
                request,
                name,
                loss,
            } => self.apply_branch_needs_confirmation(request, name, *loss),
            Event::IgnoreListed { request, result } => self.apply_ignore_listed(request, result),
            Event::IgnoreReasoned {
                request,
                path,
                result,
            } => self.apply_ignore_reasoned(request, &path, result),
            Event::IgnoreEdited { request, result } => self.apply_ignore_edited(request, result),
            Event::Identity {
                request,
                result,
                signing,
            } => {
                if request != self.identity_request {
                    return false;
                }
                self.identity_shown = Some(IdentityShown {
                    profile: self.active_profile().map(|p| p.label.clone()),
                    result,
                    signing,
                });
                true
            }
            Event::Signing { request, check } => {
                if request != self.signing_request {
                    return false;
                }
                self.form.signing_check = Some(check);
                true
            }
            Event::Staged { request, result } => {
                if request != self.stage_request {
                    return false;
                }
                self.writing = false;
                self.clear_pick();
                self.notice = Some(self.describe_stage(result));
                true
            }
            Event::Copied {
                request,
                text,
                lines,
                truncated,
            } => {
                if request != self.copy_request {
                    return false;
                }
                self.copied = Some((text, lines, truncated));
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
                let chunk = start / CHUNK_LINES;
                let doc = self.docs.entry((view, index)).or_default();
                if !doc.requested.contains(&chunk) {
                    return false;
                }
                doc.chunks.insert(chunk, lines);
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
            DiffSide::Commit => ViewKind::Commit,
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
        let rows = &(rows.start..rows.end.min(self.layout.total as usize));
        if self.files.is_empty() || rows.is_empty() {
            return None;
        }
        let (Some((first, first_local)), Some((last, last_local))) = (
            self.locate(rows.start as u64),
            self.locate(rows.end.saturating_sub(1) as u64),
        ) else {
            return None;
        };
        self.visible_chunks.clear();
        let from = first.saturating_sub(PREFETCH_BEFORE);
        let to = (last + PREFETCH_AFTER).min(self.files.len() - 1);
        for position in (from..=to).filter(|p| !(first..=last).contains(p)) {
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
            let index = self.files[position];
            let ranges = match self.split_index(index) {
                Some(split) => split.line_ranges(start..end),
                None => std::iter::once(start..end + 1).collect(),
            };
            for range in ranges {
                let (low, high) = (
                    range.start / CHUNK_LINES,
                    range.end.saturating_sub(1) / CHUNK_LINES,
                );
                let span = self.visible_chunks.entry(index).or_insert((low, high));
                *span = (span.0.min(low), span.1.max(high));
                for chunk in low..=high {
                    self.request_chunk(position, chunk);
                }
            }
        }
        Some((first, last))
    }

    pub fn loaded_lines(&self) -> usize {
        self.docs
            .values()
            .flat_map(|d| d.chunks.values())
            .map(Vec::len)
            .sum()
    }

    fn trim_loaded(&mut self, first: usize, last: usize) {
        if self.loaded_lines() <= self.max_loaded_lines {
            return;
        }
        let near = first.saturating_sub(KEEP_FILES)..=(last + KEEP_FILES);
        let positions: HashMap<usize, usize> = self
            .files
            .iter()
            .enumerate()
            .map(|(position, &index)| (index, position))
            .collect();
        let view = self.view;
        for ((doc_view, index), doc) in self.docs.iter_mut() {
            let keep = match (*doc_view == view, positions.get(index)) {
                (true, Some(position)) if (first..=last).contains(position) => {
                    let (low, high) = self.visible_chunks.get(index).copied().unwrap_or((0, 0));
                    Some(low.saturating_sub(KEEP_CHUNKS)..=high + KEEP_CHUNKS)
                }
                (true, Some(position)) if near.contains(position) => Some(0..=KEEP_CHUNKS),
                _ => None,
            };
            doc.chunks
                .retain(|chunk, _| keep.as_ref().is_some_and(|k| k.contains(chunk)));
            doc.requested
                .retain(|chunk| keep.as_ref().is_some_and(|k| k.contains(chunk)));
        }
    }

    pub fn on_visible(&mut self, rows: Range<usize>) -> bool {
        self.visible = rows.clone();
        let Some((first, last)) = self.request_range(&rows) else {
            return false;
        };
        self.trim_loaded(first, last);
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

    pub fn reading_anchor(&self) -> Option<(usize, Option<u64>)> {
        let row = self.visible.start + self.visible.len() * READING_LINE_PERCENT / 100;
        let (position, local) = self.locate(row as u64)?;
        if local == 0 {
            return Some((position, None));
        }
        let index = self.files[position];
        let line = match self.split_index(index) {
            Some(split) => {
                let cells = split.cells(local - 1)?;
                cells.left.or(cells.right)?
            }
            None => local - 1,
        };
        Some((position, Some(line)))
    }

    pub fn set_mode(&mut self, mode: DiffMode) {
        self.mode = mode;
        self.relayout();
    }

    pub fn toggle_mode(&mut self, tx: &mut Tx) {
        let anchor = self.reading_anchor();
        self.mode = match self.mode {
            DiffMode::Unified => DiffMode::Split,
            DiffMode::Split => DiffMode::Unified,
        };
        let _ = self.prefs.set(MODE_KEY, self.mode.pref());
        self.stick_end = false;
        self.relayout();
        let Some((position, line)) = anchor else {
            return;
        };
        let Some((index, _)) = self.entry_at(position) else {
            return;
        };
        let key = match line {
            None => format!("h{index}"),
            Some(line) => {
                let row = self
                    .split_index(index)
                    .and_then(|split| split.row_of_line(line))
                    .unwrap_or(line);
                format!("l{index}:{row}")
            }
        };
        tx.scroll_key_to(key, 0.0);
    }

    pub fn stick_to_end(&mut self) {
        self.stick_end = true;
        self.stick_grace = STICK_GRACE_CALLS;
    }

    pub fn step_file(&mut self, delta: isize) -> Option<usize> {
        if self.files.is_empty() {
            return None;
        }
        let last = self.files.len() - 1;
        let from = self.current.unwrap_or(0);
        let target = from.saturating_add_signed(delta).min(last);
        self.jump_to(target);
        let index = self.entry_at(target).map(|(index, _)| index)?;
        self.follow = Some(index);
        Some(index)
    }

    pub fn start_copy(
        &mut self,
        first: SelectionAnchor,
        last: SelectionAnchor,
        split_left: Option<bool>,
    ) -> bool {
        let (Some(from), Some(to)) = (
            self.files.iter().position(|&i| i == first.index),
            self.files.iter().position(|&i| i == last.index),
        ) else {
            return false;
        };
        let Some(generation) = self.repo.as_ref().map(|r| r.generation) else {
            return false;
        };
        if from > to {
            return false;
        }
        let files: Vec<_> = self.files[from..=to]
            .iter()
            .filter_map(|&index| self.entries().get(index).map(|e| (index, e.clone())))
            .collect();
        self.copy_request += 1;
        self.notice = Some("Copie en cours…".to_owned());
        if let Some(engine) = &self.engine {
            engine.send(Command::Copy(Box::new(CopyRequest {
                request: self.copy_request,
                generation,
                side: self.view.side(),
                files,
                from_row: first.row,
                from_offset: first.offset,
                to_row: last.row,
                to_offset: last.offset,
                split_left,
                max_bytes: self.copy_limit,
            })));
        }
        true
    }

    #[cfg(test)]
    pub fn peek_copied(&self) -> Option<&(String, u64, bool)> {
        self.copied.as_ref()
    }

    pub fn apply_copy(&mut self, tx: &mut Tx) {
        let Some((text, lines, truncated)) = self.copied.take() else {
            return;
        };
        tx.copy(text);
        self.notice = Some(match (truncated, lines) {
            (true, _) => {
                format!("{lines} lignes copiées : la sélection dépasse la limite de copie.")
            }
            (false, 1) => "1 ligne copiée.".to_owned(),
            (false, n) => format!("{n} lignes copiées."),
        });
    }

    pub fn perf_frame(&mut self, t: f64, tx: &mut Tx) {
        let ready = self.layout.total > 0 && self.docs.values().any(|d| d.meta.is_some());
        if let Some(perf) = self.perf.as_mut() {
            perf.frame(
                t,
                ready,
                [self.window_width, self.scroll_probe.get()[2]],
                tx,
            );
        }
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

impl State {
    pub fn capture_anchor(&self) -> Option<ReadingAnchor> {
        if let Some(pending) = &self.pending_anchor {
            return Some(pending.anchor.clone());
        }
        let (position, line) = self.reading_anchor()?;
        let (_, entry) = self.entry_at(position)?;
        Some(ReadingAnchor {
            path: entry.path.clone(),
            line,
        })
    }

    fn current_paths(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter_map(|&i| self.entries().get(i).map(|e| e.path.clone()))
            .collect()
    }

    fn set_pending(&mut self, anchor: ReadingAnchor) {
        self.pending_anchor = Some(PendingAnchor {
            anchor,
            stable_frames: 0,
            frames_left: ANCHOR_FRAMES,
        });
    }

    fn relocate(&mut self, anchor: Option<ReadingAnchor>, old_paths: &[PathBuf], announce: bool) {
        let Some(anchor) = anchor else { return };
        let new_paths = self.current_paths();
        if new_paths.contains(&anchor.path) {
            self.set_pending(anchor);
            return;
        }
        let fallback = old_paths
            .iter()
            .position(|p| *p == anchor.path)
            .and_then(|at| {
                old_paths[at + 1..]
                    .iter()
                    .chain(old_paths[..at].iter().rev())
                    .find(|p| new_paths.contains(p))
                    .cloned()
            });
        match fallback {
            Some(path) => {
                if announce {
                    self.notice = Some(format!(
                        "{} n'a plus de changement dans cette vue : lecture reprise sur {}.",
                        display_label(&anchor.path),
                        display_label(&path)
                    ));
                }
                self.set_pending(ReadingAnchor { path, line: None });
            }
            None => {
                if announce && !new_paths.is_empty() {
                    self.notice = Some(format!(
                        "{} n'a plus de changement dans cette vue.",
                        display_label(&anchor.path)
                    ));
                }
            }
        }
    }

    pub fn filter_changed(&mut self) {
        let anchor = self.capture_anchor();
        let old_paths = self.current_paths();
        self.refilter();
        self.relocate(anchor, &old_paths, false);
    }

    pub(crate) fn request_refresh(&mut self, announce: bool) {
        if self.repo.is_none() {
            return;
        }
        self.refresh_request += 1;
        self.last_refresh = Some(Instant::now());
        if announce {
            self.notice = Some(REFRESHING_NOTICE.to_owned());
        }
        if let Some(engine) = &self.engine {
            engine.send(Command::Refresh {
                request: self.refresh_request,
            });
        }
    }

    pub fn refresh(&mut self) {
        self.request_refresh(true);
    }

    pub fn refresh_on_focus(&mut self) {
        if self
            .last_refresh
            .is_none_or(|at| at.elapsed() >= FOCUS_REFRESH_MIN_INTERVAL)
        {
            self.request_refresh(false);
        }
    }

    pub fn apply_refresh(
        &mut self,
        request: u64,
        snapshot: StatusSnapshot,
        events_lost: bool,
    ) -> bool {
        let Some(current_generation) = self.repo.as_ref().map(|r| r.generation) else {
            return false;
        };
        if snapshot.generation <= current_generation {
            return false;
        }
        let anchor = self.capture_anchor();
        let old_paths = self.current_paths();
        if let Some(repo) = self.repo.as_mut() {
            repo.generation = snapshot.generation;
            repo.snapshot = Arc::new(snapshot);
        }
        self.docs.clear();
        self.current = None;
        self.pinned = None;
        self.stick_end = false;
        self.prune_marks();
        self.clear_pick();
        self.refilter();
        if self.notice.as_deref() == Some(REFRESHING_NOTICE) {
            self.notice = None;
        }
        let announce = self.notice.is_none();
        self.relocate(anchor, &old_paths, announce);
        self.reload_history_if_head_moved();
        self.probe_operation();
        if self.conflicts.open && !self.conflicts.busy {
            if self.conflicts.loading {
                self.conflicts.reload_pending = true;
            } else {
                self.reload_conflicts();
            }
        }
        if self.notice.is_none() && (request != 0 || events_lost) {
            self.notice = Some(if events_lost {
                "Des événements ont été perdus : le dépôt a été relu en entier.".to_owned()
            } else {
                "Dépôt actualisé.".to_owned()
            });
        }
        true
    }

    pub fn apply_anchor(&mut self, tx: &mut Tx) {
        let Some(pending) = self.pending_anchor.take() else {
            return;
        };
        if pending.frames_left == 0 {
            return;
        }
        let Some(position) = self.files.iter().position(|&i| {
            self.entries()
                .get(i)
                .is_some_and(|e| e.path == pending.anchor.path)
        }) else {
            return;
        };
        let Some((index, _)) = self.entry_at(position) else {
            return;
        };
        let mut pending = PendingAnchor {
            frames_left: pending.frames_left - 1,
            ..pending
        };
        let (key, target_row) = match pending.anchor.line {
            None => (format!("h{index}"), self.layout.offsets[position]),
            Some(line) => {
                if self.doc(index).and_then(|d| d.meta.as_ref()).is_none() {
                    self.pending_anchor = Some(pending);
                    return;
                }
                let row = self
                    .split_index(index)
                    .and_then(|split| split.row_of_line(line))
                    .unwrap_or(line);
                (
                    format!("l{index}:{row}"),
                    self.layout.offsets[position] + 1 + row,
                )
            }
        };
        let reading_rows = self.visible.len() * READING_LINE_PERCENT / 100;
        let expected_start = target_row.saturating_sub(reading_rows as u64);
        let at_target =
            !self.layout_dirty && (self.visible.start as u64).abs_diff(expected_start) <= 1;
        pending.stable_frames = if at_target {
            pending.stable_frames + 1
        } else {
            0
        };
        if pending.stable_frames >= ANCHOR_STABLE_FRAMES {
            return;
        }
        if !at_target {
            tx.scroll_key_to(key, reading_rows as f32 * crate::view::LINE_HEIGHT);
        }
        self.pending_anchor = Some(pending);
    }

    pub fn shutdown(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }
}

impl State {
    fn describe_stage(&mut self, result: Result<StageOutcome, WriteError>) -> String {
        match result {
            Ok(StageOutcome::Applied { staged }) => {
                self.clear_marks();
                match staged {
                    0 => "Rien à indexer.".to_owned(),
                    1 => "1 fichier indexé.".to_owned(),
                    n => format!("{n} fichiers indexés."),
                }
            }
            Ok(StageOutcome::Unstaged { files }) => {
                self.clear_marks();
                match files {
                    0 => "Rien à désindexer.".to_owned(),
                    1 => "1 fichier désindexé.".to_owned(),
                    n => format!("{n} fichiers désindexés."),
                }
            }
            Ok(StageOutcome::HunkApplied) => "Bloc indexé.".to_owned(),
            Ok(StageOutcome::HunkUnstaged) => "Bloc désindexé.".to_owned(),
            Ok(StageOutcome::LinesApplied { lines }) => match lines {
                1 => "1 ligne indexée.".to_owned(),
                n => format!("{n} lignes indexées."),
            },
            Ok(StageOutcome::LinesUnstaged { lines }) => match lines {
                1 => "1 ligne désindexée.".to_owned(),
                n => format!("{n} lignes désindexées."),
            },
            Ok(StageOutcome::Refused { reason, path }) => match reason {
                Refusal::Conflict => format!(
                    "{} est en conflit : la résolution n'est pas encore disponible, rien n'a été modifié.",
                    display_label(&path)
                ),
                Refusal::NothingToStage => format!("{} n'a rien à indexer.", display_label(&path)),
                Refusal::NothingToUnstage => {
                    format!("{} n'a rien à désindexer.", display_label(&path))
                }
                Refusal::NotPartial => "Ce fichier ne peut pas être traité bloc par bloc ou ligne par ligne (non suivi, supprimé, renommé ou changement de mode) : traite le fichier entier.".to_owned(),
                Refusal::FilterConfigured => "Ce dépôt utilise des filtres Git : l'indexation par bloc ou par ligne est désactivée pour ne pas indexer un contenu différent de celui affiché. Indexe le fichier entier.".to_owned(),
                Refusal::NothingSelected => {
                    "La sélection ne contient aucune ligne ajoutée ou supprimée.".to_owned()
                }
                Refusal::Incompatible => "Cette sélection est impossible dans un bloc qui touche la fin d'un fichier sans saut de ligne : sélectionne tout le bloc.".to_owned(),
            },
            Ok(StageOutcome::Stale { paths }) => format!(
                "Rien n'a été modifié : {} a changé depuis son affichage. L'état a été relu, vérifie puis recommence.",
                paths.first().map_or_else(String::new, |p| display_label(p))
            ),
            Err(WriteError::IndexLocked(path)) => format!(
                "Git est occupé : le verrou {path} existe. Termine l'autre opération Git puis réessaie ; GitPoulpe ne supprime jamais ce verrou."
            ),
            Err(WriteError::NoWorktree) => {
                "Ce dépôt n'a pas de répertoire de travail.".to_owned()
            }
            Err(WriteError::Cancelled) => "Opération annulée.".to_owned(),
            Err(WriteError::GitUnavailable(d)) => format!("Git est introuvable : {d}"),
            Err(WriteError::GitFailed(d)) => format!("L'opération a échoué : {d}"),
        }
    }

    fn prune_marks(&mut self) {
        let view = self.view;
        let still: HashSet<PathBuf> = self
            .entries()
            .iter()
            .filter(|e| Self::in_view(e, view) && !e.is_conflict())
            .map(|e| e.path.clone())
            .collect();
        let before = self.marked.len();
        self.marked.retain(|path| still.contains(path));
        if self.marked.len() != before {
            self.marks_version += 1;
        }
    }

    fn clear_marks(&mut self) {
        if !self.marked.is_empty() {
            self.marked.clear();
            self.marks_version += 1;
        }
    }

    pub fn can_mark(&self, position: usize) -> bool {
        self.view != ViewKind::Commit
            && self
                .entry_at(position)
                .is_some_and(|(_, entry)| Self::in_view(entry, self.view) && !entry.is_conflict())
    }

    pub fn toggle_mark(&mut self, position: usize) {
        if !self.can_mark(position) {
            return;
        }
        let Some((_, entry)) = self.entry_at(position) else {
            return;
        };
        let path = entry.path.clone();
        if !self.marked.remove(&path) {
            self.marked.insert(path);
        }
        self.marks_version += 1;
    }

    pub fn clear_pick(&mut self) {
        if self.line_pick.take().is_some() {
            self.pick_version += 1;
        }
    }

    pub fn is_picked(&self, index: usize, doc_line: u64) -> bool {
        self.line_pick.as_ref().is_some_and(|pick| {
            pick.index == index
                && pick
                    .spans
                    .iter()
                    .any(|(from, to)| (*from..=*to).contains(&doc_line))
        })
    }

    pub fn pick_line(&mut self, index: usize, doc_line: u64, extend: bool) {
        let mut pick = match self.line_pick.take() {
            Some(pick) if pick.index == index => pick,
            _ => LinePick {
                index,
                anchor: doc_line,
                spans: Vec::new(),
            },
        };
        if extend && !pick.spans.is_empty() {
            let (from, to) = (pick.anchor.min(doc_line), pick.anchor.max(doc_line));
            pick.spans.push((from, to));
        } else if let Some(at) = pick.spans.iter().position(|s| *s == (doc_line, doc_line)) {
            pick.spans.remove(at);
            pick.anchor = doc_line;
        } else {
            pick.spans.push((doc_line, doc_line));
            pick.anchor = doc_line;
        }
        self.line_pick = (!pick.spans.is_empty()).then_some(pick);
        self.pick_version += 1;
    }

    fn target_for(&self, path: &Path) -> Option<StageTarget> {
        let repo = self.repo.as_ref()?;
        let entry = repo.snapshot.entries.iter().find(|e| e.path == path)?;
        Some(StageTarget {
            stamp: repo.snapshot.stamps.get(path).copied(),
            index_id: repo.snapshot.index_ids.get(path).cloned(),
            entry: entry.clone(),
        })
    }

    fn submit(&mut self, job: WriteJob) {
        if self.writing {
            self.notice = Some("Une écriture est déjà en cours.".to_owned());
            return;
        }
        self.stage_request += 1;
        self.writing = true;
        self.notice = Some("Opération en cours…".to_owned());
        if let Some(engine) = &self.engine {
            engine.send(Command::Write {
                request: self.stage_request,
                job: Box::new(job),
            });
        }
    }

    fn hunk_spec(&self, entry_index: usize, hunk_index: usize) -> Option<HunkSpec> {
        let hunk = self
            .doc(entry_index)
            .and_then(|d| d.meta.as_ref())
            .and_then(|m| m.hunks.get(hunk_index))?;
        Some(HunkSpec {
            index: hunk_index,
            old_start: hunk.old_start,
            old_len: hunk.old_len,
            new_start: hunk.new_start,
            new_len: hunk.new_len,
        })
    }

    pub fn stage_current(&mut self) {
        if self.view != ViewKind::Unstaged {
            return;
        }
        if let Some((_, entry)) = self.current.and_then(|p| self.entry_at(p)) {
            let path = entry.path.clone();
            self.stage_paths(vec![path]);
        }
    }

    pub fn unstage_current(&mut self) {
        if self.view != ViewKind::Staged {
            return;
        }
        if let Some((_, entry)) = self.current.and_then(|p| self.entry_at(p)) {
            let path = entry.path.clone();
            self.unstage_paths(vec![path]);
        }
    }

    pub fn write_marked(&mut self) {
        let mut paths: Vec<PathBuf> = self.marked.iter().cloned().collect();
        paths.sort();
        match self.view {
            ViewKind::Unstaged => self.stage_paths(paths),
            ViewKind::Staged => self.unstage_paths(paths),
            ViewKind::Commit => {}
        }
    }

    pub fn stage_paths(&mut self, paths: Vec<PathBuf>) {
        let targets: Vec<StageTarget> = paths.iter().filter_map(|p| self.target_for(p)).collect();
        if targets.is_empty() {
            self.notice = Some("Rien à indexer.".to_owned());
            return;
        }
        self.submit(WriteJob::Stage(targets));
    }

    pub fn unstage_paths(&mut self, paths: Vec<PathBuf>) {
        let targets: Vec<StageTarget> = paths.iter().filter_map(|p| self.target_for(p)).collect();
        if targets.is_empty() {
            self.notice = Some("Rien à désindexer.".to_owned());
            return;
        }
        self.submit(WriteJob::Unstage(targets));
    }

    pub fn write_hunk(&mut self, path: PathBuf, hunk_index: usize) {
        let Some(position) = self.entries().iter().position(|e| e.path == path) else {
            return;
        };
        let (Some(target), Some(hunk)) =
            (self.target_for(&path), self.hunk_spec(position, hunk_index))
        else {
            self.notice = Some("Ce bloc n'est plus affiché : l'état va être relu.".to_owned());
            return;
        };
        self.submit(match self.view {
            ViewKind::Unstaged => WriteJob::StageHunk { target, hunk },
            ViewKind::Staged => WriteJob::UnstageHunk { target, hunk },
            ViewKind::Commit => return,
        });
    }

    pub fn write_picked_lines(&mut self) {
        let Some(pick) = self.line_pick.clone() else {
            return;
        };
        let Some(entry) = self.entries().get(pick.index) else {
            return;
        };
        let path = entry.path.clone();
        let Some(meta) = self.doc(pick.index).and_then(|d| d.meta.as_ref()) else {
            return;
        };
        let mut hunks = Vec::new();
        for (rank, hunk) in meta.hunks.iter().enumerate() {
            let body_from = hunk.first_line + 1;
            let body_to = hunk.first_line + hunk.line_count.saturating_sub(1);
            let ranges: Vec<(u32, u32)> = pick
                .spans
                .iter()
                .filter_map(|(from, to)| {
                    let (from, to) = ((*from).max(body_from), (*to).min(body_to));
                    (from <= to).then(|| {
                        (
                            u32::try_from(from - body_from).unwrap_or(u32::MAX),
                            u32::try_from(to - body_from).unwrap_or(u32::MAX),
                        )
                    })
                })
                .collect();
            if !ranges.is_empty() {
                hunks.push(HunkLines {
                    hunk: HunkSpec {
                        index: rank,
                        old_start: hunk.old_start,
                        old_len: hunk.old_len,
                        new_start: hunk.new_start,
                        new_len: hunk.new_len,
                    },
                    ranges,
                });
            }
        }
        if hunks.is_empty() {
            self.notice =
                Some("La sélection ne contient aucune ligne ajoutée ou supprimée.".to_owned());
            return;
        }
        let Some(target) = self.target_for(&path) else {
            return;
        };
        let selection = LineSelection { hunks };
        self.submit(match self.view {
            ViewKind::Unstaged => WriteJob::StageLines { target, selection },
            ViewKind::Staged => WriteJob::UnstageLines { target, selection },
            ViewKind::Commit => return,
        });
    }
}

impl State {
    pub fn repo_key(&self) -> Option<String> {
        let repo = self.repo.as_ref()?;
        Some(ProfileBook::repo_key(repo.snapshot.repo.common_dir()))
    }

    pub fn active_profile(&self) -> Option<&Profile> {
        self.book.profile_for(&self.repo_key()?)
    }

    pub fn request_identity(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.identity_request += 1;
        let profile = self.active_profile().cloned().map(Box::new);
        if let Some(engine) = &self.engine {
            engine.send(Command::Identity {
                request: self.identity_request,
                profile,
            });
        }
    }

    fn request_form_signing_check(&mut self, profile: &Profile) {
        self.form.signing_check = None;
        if profile.signing == gitpoulpe_core::SigningPreference::None {
            self.form.signing_check = Some(SigningCheck::NotRequested);
            return;
        }
        self.signing_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::CheckSigning {
                request: self.signing_request,
                profile: Box::new(profile.clone()),
            });
        }
    }

    fn persist_profiles(&mut self) {
        let _ = self.prefs.set(PROFILES_KEY, &self.book);
    }

    pub fn open_profiles(&mut self) {
        self.profiles_open = true;
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        let selected = self
            .active_profile()
            .or_else(|| self.book.profiles.first())
            .cloned();
        match selected {
            Some(profile) => self.edit_profile(&profile.id),
            None => self.new_profile(),
        }
    }

    pub fn close_profiles(&mut self) {
        self.profiles_open = false;
    }

    pub fn new_profile(&mut self) {
        self.form = ProfileForm::default();
    }

    pub fn edit_profile(&mut self, id: &str) {
        let Some(profile) = self.book.get(id).cloned() else {
            return;
        };
        self.form.load(&profile);
        self.request_form_signing_check(&profile);
    }

    pub fn set_form_signing(&mut self, kind: SigningKind) {
        self.form.signing = kind;
        self.form.signing_check = None;
    }

    pub fn save_profile(&mut self) {
        let id = self
            .form
            .editing
            .clone()
            .unwrap_or_else(|| self.book.new_id());
        let profile = self.form.to_profile(id.clone());
        if let Err(error) = self.book.upsert(profile) {
            self.form.error = Some(error.to_string());
            return;
        }
        self.persist_profiles();
        let saved = self.book.get(&id).cloned();
        self.form.error = None;
        self.form.editing = Some(id);
        if let Some(saved) = saved {
            self.request_form_signing_check(&saved);
        }
        self.request_identity();
        self.notice = Some("Profil enregistré.".to_owned());
    }

    pub fn delete_profile(&mut self) {
        let Some(id) = self.form.editing.clone() else {
            return;
        };
        self.book.remove(&id);
        self.persist_profiles();
        self.form = ProfileForm::default();
        self.request_identity();
        self.notice = Some("Profil supprimé.".to_owned());
    }

    pub fn use_profile_here(&mut self, id: Option<String>) {
        let Some(key) = self.repo_key() else {
            return;
        };
        match self.book.assign(&key, id.as_deref()) {
            Ok(()) => {
                self.persist_profiles();
                self.request_identity();
                self.notice = Some(match id {
                    Some(_) => "Profil appliqué à ce dépôt.".to_owned(),
                    None => "Ce dépôt suit de nouveau la configuration Git.".to_owned(),
                });
            }
            Err(error) => self.form.error = Some(error.to_string()),
        }
    }
}

impl State {
    pub fn staged_for_commit(&self) -> Vec<(PathBuf, String)> {
        let Some(repo) = self.repo.as_ref() else {
            return Vec::new();
        };
        repo.snapshot
            .entries
            .iter()
            .filter(|e| e.is_staged())
            .map(|e| {
                (
                    e.path.clone(),
                    repo.snapshot
                        .index_ids
                        .get(&e.path)
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    pub fn set_amend_mode(&mut self, on: bool) {
        if on == self.amend_mode || self.repo.is_none() {
            return;
        }
        self.amend_mode = on;
        self.amend_confirming = false;
        if on {
            self.amend_info = None;
            self.amend_info_request += 1;
            if let Some(engine) = &self.engine {
                engine.send(Command::AmendInfo {
                    request: self.amend_info_request,
                });
            }
        } else {
            if self
                .amend_info
                .as_ref()
                .is_some_and(|info| info.message.trim_end() == self.commit_message.text.trim_end())
            {
                self.commit_message = TextEdit::new("");
            }
            self.amend_info = None;
        }
    }

    pub fn commit(&mut self) {
        if self.amend_mode {
            self.amend();
            return;
        }
        if self.writing {
            self.notice = Some("Une écriture est déjà en cours.".to_owned());
            return;
        }
        if self.repo.is_none() {
            return;
        }
        let request = CommitRequest {
            message: self.commit_message.text.clone(),
            staged: self.staged_for_commit(),
            profile: self.active_profile().cloned(),
        };
        self.commit_request += 1;
        self.writing = true;
        self.submitted_amend = false;
        self.commit_report = None;
        self.notice = Some("Création du commit…".to_owned());
        if let Some(engine) = &self.engine {
            engine.send(Command::Commit {
                request: self.commit_request,
                job: Box::new(request),
            });
        }
    }

    fn amend(&mut self) {
        if self.writing {
            self.notice = Some("Une écriture est déjà en cours.".to_owned());
            return;
        }
        let Some(info) = self.amend_info.clone() else {
            self.notice = Some("Lecture du dernier commit en cours…".to_owned());
            return;
        };
        if !info.published_on.is_empty() && !self.amend_confirming {
            self.amend_confirming = true;
            self.notice = Some(
                "Ce commit est déjà publié : lis l'avertissement puis confirme pour l'amender."
                    .to_owned(),
            );
            return;
        }
        let request = AmendRequest {
            commit: CommitRequest {
                message: self.commit_message.text.clone(),
                staged: self.staged_for_commit(),
                profile: self.active_profile().cloned(),
            },
            head: info.head,
            confirmed_published: self.amend_confirming,
        };
        self.commit_request += 1;
        self.writing = true;
        self.submitted_amend = true;
        self.commit_report = None;
        self.notice = Some("Amendement du commit…".to_owned());
        if let Some(engine) = &self.engine {
            engine.send(Command::Amend {
                request: self.commit_request,
                job: Box::new(request),
            });
        }
    }

    pub(crate) fn record_journal(
        &mut self,
        action: &str,
        before: Option<String>,
        after: Option<String>,
        detail: String,
    ) {
        let Some(key) = self.repo_key() else { return };
        let prefs_key = format!("journal:{key}");
        let mut journal: Vec<JournalEntry> = self.prefs.get(&prefs_key).unwrap_or_default();
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        push_journal(
            &mut journal,
            JournalEntry {
                at,
                action: action.to_owned(),
                before,
                after,
                detail,
            },
        );
        let _ = self.prefs.set(&prefs_key, &journal);
    }

    #[cfg(test)]
    pub fn journal(&self) -> Vec<JournalEntry> {
        self.repo_key()
            .and_then(|key| self.prefs.get(&format!("journal:{key}")))
            .unwrap_or_default()
    }

    fn finish_commit(&mut self, result: Result<CommitOutcome, WriteError>) {
        match result {
            Ok(CommitOutcome::Created {
                oid,
                subject,
                before,
                signature,
            }) => {
                self.commit_message = TextEdit::new("");
                self.commit_report = None;
                let short: String = oid.chars().take(8).collect();
                let signed_note = match signature {
                    Some(gitpoulpe_core::Signature {
                        kind: gitpoulpe_core::SignatureKind::Ssh,
                        verified: true,
                    }) => " et signé en SSH (signature vérifiée)",
                    Some(gitpoulpe_core::Signature {
                        kind: gitpoulpe_core::SignatureKind::Ssh,
                        ..
                    }) => " et signé en SSH",
                    Some(gitpoulpe_core::Signature {
                        kind: gitpoulpe_core::SignatureKind::Gpg,
                        verified: true,
                    }) => " et signé en GPG (signature vérifiée)",
                    Some(gitpoulpe_core::Signature {
                        kind: gitpoulpe_core::SignatureKind::Gpg,
                        ..
                    }) => " et signé en GPG",
                    None => "",
                };
                if self.submitted_amend {
                    let previous: String = before
                        .as_deref()
                        .unwrap_or_default()
                        .chars()
                        .take(8)
                        .collect();
                    self.notice = Some(format!(
                        "Commit amendé{signed_note} : {short} remplace {previous} (toujours récupérable : journal, `git reflog`) — {subject}"
                    ));
                    self.amend_mode = false;
                    self.amend_info = None;
                    self.amend_confirming = false;
                    self.record_journal("amend", before, Some(oid), subject);
                } else {
                    self.notice = Some(format!("Commit {short} créé{signed_note} : {subject}"));
                    self.record_journal("commit", before, Some(oid), subject);
                }
            }
            Ok(CommitOutcome::Refused(reason)) => {
                if self.amend_mode {
                    match reason {
                        CommitRefusal::Published => self.amend_confirming = true,
                        CommitRefusal::Stale => {
                            self.amend_info = None;
                            self.amend_confirming = false;
                            self.amend_info_request += 1;
                            if let Some(engine) = &self.engine {
                                engine.send(Command::AmendInfo {
                                    request: self.amend_info_request,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                self.notice = Some(
                    match reason {
                        CommitRefusal::EmptyMessage => "Écris un message de commit.",
                        CommitRefusal::EmptyIndex => "Rien n'est indexé : indexe des changements avant de créer le commit.",
                        CommitRefusal::Conflicts => "Des conflits ne sont pas résolus : ouvrez « Résoudre » pour les traiter, aucun commit n'a été créé.",
                        CommitRefusal::Stale => "L'index ou le dernier commit a changé depuis son affichage : l'état a été relu, vérifie puis recommence. Ton message est conservé.",
                        CommitRefusal::NoCommit => "Il n'y a aucun commit à amender.",
                        CommitRefusal::NothingToAmend => "Rien à amender : le message est inchangé et aucun changement n'est indexé.",
                        CommitRefusal::Published => "Ce commit est déjà publié : confirme l'amendement pour continuer.",
                    }
                    .to_owned(),
                );
            }
            Ok(CommitOutcome::Failed { kind, output }) => {
                let lead = match kind {
                    CommitFailureKind::IdentityUnknown => {
                        "Git ne connaît pas ton identité. Choisis un profil (bouton Profil) ou configure user.name et user.email."
                    }
                    CommitFailureKind::SigningUnavailable => {
                        "La signature a échoué : le commit n'a pas été créé."
                    }
                    CommitFailureKind::Hook => {
                        "Un hook Git a refusé le commit. Corrige le problème puis réessaie."
                    }
                    CommitFailureKind::Other => "Git a refusé le commit.",
                };
                self.notice = Some(format!("{lead} Ton message est conservé."));
                self.commit_report = Some(format!("{lead}\n{output}"));
            }
            Err(error) => {
                self.notice = Some(self.describe_stage(Err(error)));
            }
        }
    }
}

impl State {
    fn head_oid(&self) -> Option<String> {
        self.repo
            .as_ref()
            .and_then(|r| r.snapshot.branch.oid.clone())
    }

    pub fn open_history(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.history_open = true;
        self.profiles_open = false;
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.start_history();
    }

    fn start_history(&mut self) {
        self.history_request += 1;
        let head = self.head_oid();
        self.history.restart(self.history_request, head);
        if let Some(engine) = &self.engine {
            engine.send(Command::HistoryOpen {
                request: self.history_request,
            });
        }
    }

    pub fn close_history(&mut self) {
        if !self.history_open {
            return;
        }
        self.history_open = false;
        self.history = HistoryState::default();
        self.search_text = TextEdit::new("");
        self.search.clear();
        self.pending_reveal = None;
        if let Some(engine) = &self.engine {
            engine.send(Command::HistoryClose);
        }
    }

    pub fn history_visible(&mut self, rows: std::ops::Range<usize>) -> bool {
        if !self.history.needs_more(rows.end) {
            return false;
        }
        self.history.loading = true;
        if let Some(engine) = &self.engine {
            engine.send(Command::HistoryMore {
                request: self.history.request,
            });
        }
        false
    }

    pub fn select_commit(&mut self, index: usize) {
        if index < self.history.commits.len() {
            self.history.selected = Some(index);
            self.history.version += 1;
        }
    }

    pub fn reload_history_if_head_moved(&mut self) {
        if self.history_open && self.history.head != self.head_oid() {
            self.start_history();
        }
    }
}

impl State {
    pub fn search_changed(&mut self) {
        let text = self.search_text.text.trim().to_owned();
        if text.is_empty() {
            self.search.clear();
            if let Some(engine) = &self.engine {
                engine.send(Command::SearchClose);
            }
            return;
        }
        let request = self.search.request + 1;
        self.search.restart(request);
        if let Some(engine) = &self.engine {
            engine.send(Command::SearchOpen { request, text });
        }
    }

    pub fn search_active(&self) -> bool {
        !self.search_text.text.trim().is_empty()
    }

    pub fn search_visible(&mut self, rows: std::ops::Range<usize>) -> bool {
        if !self.search.needs_more(rows.end) {
            return false;
        }
        self.search.loading = true;
        if let Some(engine) = &self.engine {
            engine.send(Command::SearchMore {
                request: self.search.request,
            });
        }
        false
    }

    pub fn reveal_commit(&mut self, oid: String) {
        self.search_text = TextEdit::new("");
        self.search_changed();
        self.pending_reveal = Some(oid);
    }

    pub fn apply_reveal(&mut self, tx: &mut Tx) {
        let Some(oid) = self.pending_reveal.clone() else {
            return;
        };
        if let Some(index) = self.history.commits.iter().position(|c| c.oid == oid) {
            self.pending_reveal = None;
            self.history.selected = Some(index);
            self.history.version += 1;
            tx.scroll_key_to(format!("hk{index}"), 120.0);
            return;
        }
        if self.history.done {
            self.pending_reveal = None;
            self.notice = Some("Ce commit n'est plus dans l'historique affiché.".to_owned());
            return;
        }
        if !self.history.loading {
            self.history.loading = true;
            if let Some(engine) = &self.engine {
                engine.send(Command::HistoryMore {
                    request: self.history.request,
                });
            }
        }
    }
}

impl State {
    pub fn open_commit(&mut self, oid: String, parent_index: usize) {
        if self.repo.is_none() {
            return;
        }
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.inspect_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::CommitInspect {
                request: self.inspect_request,
                oid,
                parent_index,
            });
        }
    }

    pub fn choose_parent(&mut self, parent_index: usize) {
        let Some(oid) = self.inspected.as_ref().map(|i| i.detail.commit.oid.clone()) else {
            return;
        };
        self.open_commit(oid, parent_index);
    }

    pub fn leave_commit(&mut self) {
        if self.view != ViewKind::Commit {
            return;
        }
        self.inspected = None;
        self.view = ViewKind::Unstaged;
        self.docs.clear();
        self.current = None;
        self.pinned = None;
        self.refilter();
        if let Some(engine) = &self.engine {
            engine.send(Command::CommitLeave);
        }
        self.refresh();
        if std::mem::take(&mut self.return_to_history) {
            self.open_history();
        }
    }
}
