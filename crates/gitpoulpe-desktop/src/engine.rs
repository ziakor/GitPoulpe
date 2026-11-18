use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use gitpoulpe_core::{
    AmendInfo, AmendRequest, Branch, BranchLoss, ChangeEntry, Commit, CommitDetail, CommitOutcome,
    CommitRange, CommitRequest, ConflictFile, ConflictVersions, Connection, DiffLine, DiffSide,
    DocumentMeta, EmptyPolicy, FlowKind, FlowOutcome, FlowPlan, Generation, GitflowConfig,
    GitflowView, HooksView, IgnoreReason, IgnoreScope, LfsFile, LfsStatus, LfsTransfer, MergeMode,
    MergeOutcome, MergePreview, OpenError, Operation, OperationKind, PlanItem, Profile,
    PullOutcome, PullPreview, PullStrategy, PushMode, PushOutcome, PushPreview, RangeError,
    ReadError, RebaseOutcome, RebasePlanInfo, ReflogEntry, ReflogInspect, Remote, RemoteImpact,
    ResetMode, ResetOutcome, ResetPreview, Resolution, SequenceKind, SequenceOutcome,
    SequencePreview, SessionId, SessionInfo, StageOutcome, Stash, StashApplied, StashDetail,
    StashScope, StatusSnapshot, Submodule, Tag, UndoDirection, UndoRecord, Worktree,
    WorktreeRemoval, WriteError, WriteJob, mask_url,
};
use gitpoulpe_git::{
    BranchChoice, BranchError, CacheConfig, Cancel, CloneError, ConflictError, ConflictSide,
    DeleteMode, DiffCache, DiffError, DiffReader, FetchOutcome, Git, HistoryError, HistoryStream,
    HookError, HostKey, Identity, IdentityError, IgnoreError, IgnoredList, InspectError, MAX_WAIT,
    NetError, PullRequest, PushRequest, QUIET_PERIOD, RemoteCheck, RepoWatcher, SequenceRequest,
    Signal, SigningCheck, StashError, SubmoduleError, WorktreeError, check_signing, coalesce,
};
use gitpoulpe_hosting::{DeviceCode, Session as AccountSession};
use rune::Waker;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const MAX_COPY_BYTES: usize = 32 * 1024 * 1024;
const COPY_SPAN_LINES: u64 = 4096;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreEdit {
    Add { scope: IgnoreScope, rule: String },
    Remove { scope: IgnoreScope, rule: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchOp {
    Create { name: String },
    Switch { name: String },
    Rename { old: String, new: String },
    Delete { name: String, mode: DeleteMode },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagOp {
    Create {
        name: String,
        target: Option<String>,
        message: Option<String>,
        profile: Option<Box<Profile>>,
    },
    Delete {
        name: String,
        expected: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOp {
    Add {
        name: String,
        url: String,
    },
    SetUrl {
        name: String,
        url: String,
        expected: Vec<String>,
    },
    SetPushUrl {
        name: String,
        url: Option<String>,
        expected: Vec<String>,
    },
    Rename {
        old: String,
        new: String,
    },
    Remove {
        name: String,
        expected: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictOp {
    Side {
        path: PathBuf,
        side: ConflictSide,
    },
    Content {
        path: PathBuf,
        content: String,
        allow_markers: bool,
    },
    Revalidate {
        path: PathBuf,
    },
    Skip,
    Continue {
        profile: Option<Box<Profile>>,
    },
    Abort,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictSnapshot {
    pub operation: Option<Operation>,
    pub files: Vec<ConflictFile>,
    pub states: Vec<(PathBuf, Resolution)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictDone {
    pub message: String,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StashOp {
    Create {
        message: Option<String>,
        scope: StashScope,
        profile: Option<Box<Profile>>,
    },
    Apply {
        oid: String,
        pop: bool,
        restore_index: bool,
    },
    Drop {
        oid: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashDone {
    pub message: String,
    pub conflicts: bool,
    pub journal: Option<(String, Option<String>, Option<String>, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeOp {
    Add {
        path: PathBuf,
        branch: String,
        create: bool,
    },
    Lock {
        path: PathBuf,
        reason: String,
    },
    Unlock {
        path: PathBuf,
    },
    Remove {
        path: PathBuf,
        discard: bool,
    },
    Prune,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchDone {
    pub message: String,
    pub journal: Option<(String, Option<String>, Option<String>, String)>,
}

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
    Refresh {
        request: u64,
    },
    Write {
        request: u64,
        job: Box<WriteJob>,
    },
    Commit {
        request: u64,
        job: Box<CommitRequest>,
    },
    HistoryOpen {
        request: u64,
    },
    HistoryMore {
        request: u64,
    },
    HistoryClose,
    SearchOpen {
        request: u64,
        text: String,
    },
    SearchMore {
        request: u64,
    },
    SearchClose,
    CommitInspect {
        request: u64,
        oid: String,
        parent_index: usize,
    },
    CommitLeave,
    Amend {
        request: u64,
        job: Box<AmendRequest>,
    },
    AmendInfo {
        request: u64,
    },
    Identity {
        request: u64,
        profile: Option<Box<Profile>>,
    },
    CheckSigning {
        request: u64,
        profile: Box<Profile>,
    },
    IgnoreList {
        request: u64,
    },
    IgnoreReason {
        request: u64,
        path: PathBuf,
    },
    IgnoreEdit {
        request: u64,
        edit: IgnoreEdit,
    },
    RemoteList {
        request: u64,
    },
    RemoteOp {
        request: u64,
        op: RemoteOp,
    },
    RemoteImpact {
        request: u64,
        name: String,
    },
    RemoteCheck {
        request: u64,
        name: String,
    },
    SshScan {
        request: u64,
        host: String,
        port: Option<u16>,
    },
    SshTrust {
        request: u64,
        host: String,
        port: Option<u16>,
        fingerprint: String,
    },
    Fetch {
        request: u64,
        remote: String,
    },
    Clone {
        request: u64,
        url: String,
        parent: PathBuf,
        name: String,
    },
    OperationProbe {
        request: u64,
    },
    ConflictLoad {
        request: u64,
        remembered: Vec<PathBuf>,
    },
    ConflictVersions {
        request: u64,
        path: PathBuf,
    },
    ConflictOp {
        request: u64,
        op: ConflictOp,
        remembered: Vec<PathBuf>,
    },
    MergePreview {
        request: u64,
        source: String,
    },
    Merge {
        request: u64,
        source: String,
        mode: MergeMode,
        expected_head: String,
        autostash: bool,
        profile: Option<Box<Profile>>,
    },
    StashList {
        request: u64,
    },
    StashDetail {
        request: u64,
        oid: String,
    },
    StashOp {
        request: u64,
        op: StashOp,
    },
    PublishOpen {
        request: u64,
        branch: String,
    },
    PushPreview {
        request: u64,
        branch: String,
        remote: String,
    },
    Push {
        request: u64,
        branch: String,
        remote: String,
        mode: PushMode,
        expected: Option<String>,
        set_upstream: bool,
    },
    ReflogList {
        request: u64,
        reference: String,
    },
    ReflogInspect {
        request: u64,
        oid: String,
    },
    ReflogRecover {
        request: u64,
        oid: String,
        name: String,
    },
    ResetPreview {
        request: u64,
        target: String,
        mode: ResetMode,
    },
    Reset {
        request: u64,
        target: String,
        mode: ResetMode,
        expected_head: String,
        confirmed: bool,
    },
    SequencePreview {
        request: u64,
        kind: SequenceKind,
        oids: Vec<String>,
    },
    Sequence {
        request: u64,
        kind: SequenceKind,
        oids: Vec<String>,
        mainline: Option<u32>,
        empty: EmptyPolicy,
        expected_head: String,
        profile: Option<Box<Profile>>,
    },
    UndoRedo {
        request: u64,
        record: Box<UndoRecord>,
        direction: UndoDirection,
    },
    RebasePlan {
        request: u64,
        base: String,
    },
    Rebase {
        request: u64,
        base: String,
        items: Vec<PlanItem>,
        expected_head: String,
        autostash: bool,
        profile: Option<Box<Profile>>,
    },
    PullPreview {
        request: u64,
    },
    Pull {
        request: u64,
        strategy: PullStrategy,
        expected_head: String,
        autostash: bool,
        profile: Option<Box<Profile>>,
    },
    WorktreeList {
        request: u64,
    },
    WorktreeRemovalPreview {
        request: u64,
        path: PathBuf,
    },
    WorktreeOp {
        request: u64,
        op: WorktreeOp,
    },
    HookList {
        request: u64,
    },
    HookSet {
        request: u64,
        name: String,
        enabled: bool,
        confirm_shared: bool,
    },
    LfsLoad {
        request: u64,
    },
    LfsRun {
        request: u64,
        kind: LfsTransfer,
        branch: String,
    },
    GitflowLoad {
        request: u64,
    },
    GitflowPlan {
        request: u64,
        kind: FlowKind,
        finish: bool,
        name: String,
    },
    GitflowRun {
        request: u64,
        plan: Box<FlowPlan>,
        finish: bool,
        profile: Option<Box<Profile>>,
    },
    GitflowSave {
        request: u64,
        config: Box<GitflowConfig>,
        create_develop: bool,
    },
    SubmoduleList {
        request: u64,
    },
    SubmoduleRun {
        request: u64,
        path: PathBuf,
        init_only: bool,
        confirm: bool,
    },
    NetCancel,
    InitPrepare {
        request: u64,
        path: PathBuf,
    },
    Init {
        request: u64,
        path: PathBuf,
        branch: String,
    },
    TagList {
        request: u64,
    },
    TagOp {
        request: u64,
        op: TagOp,
    },
    BranchList {
        request: u64,
    },
    BranchOp {
        request: u64,
        op: BranchOp,
    },
    Shutdown(std::sync::mpsc::Sender<()>),
    Read {
        generation: Generation,
        side: DiffSide,
        index: usize,
        entry: Box<ChangeEntry>,
        range: Range<u64>,
    },
}

pub enum Event {
    Refreshed {
        request: u64,
        snapshot: Box<StatusSnapshot>,
        events_lost: bool,
    },
    RefreshFailed {
        request: u64,
        message: String,
    },
    Staged {
        request: u64,
        result: Result<StageOutcome, WriteError>,
    },
    Committed {
        request: u64,
        result: Result<CommitOutcome, WriteError>,
    },
    AmendInfo {
        request: u64,
        info: Option<AmendInfo>,
    },
    CommitInspected {
        request: u64,
        generation: Generation,
        detail: Box<CommitDetail>,
        base_index: usize,
        entries: Vec<ChangeEntry>,
    },
    CommitInspectFailed {
        request: u64,
        message: String,
    },
    HistoryPage {
        request: u64,
        commits: Vec<Commit>,
        done: bool,
    },
    HistoryFailed {
        request: u64,
        no_commits: bool,
        message: String,
    },
    SearchPage {
        request: u64,
        commits: Vec<Commit>,
        done: bool,
    },
    SearchFailed {
        request: u64,
        message: String,
    },
    Identity {
        request: u64,
        result: Result<Identity, String>,
        signing: SigningCheck,
    },
    Signing {
        request: u64,
        check: SigningCheck,
    },
    WatchState {
        available: bool,
        reason: String,
    },
    TagsListed {
        request: u64,
        result: Result<Vec<Tag>, String>,
    },
    TagDone {
        request: u64,
        result: Result<BranchDone, String>,
    },
    BranchesListed {
        request: u64,
        result: Result<Vec<Branch>, String>,
    },
    BranchDone {
        request: u64,
        result: Result<BranchDone, String>,
    },
    BranchNeedsConfirmation {
        request: u64,
        name: String,
        loss: Box<BranchLoss>,
    },
    IgnoreListed {
        request: u64,
        result: Result<IgnoredList, String>,
    },
    IgnoreReasoned {
        request: u64,
        path: PathBuf,
        result: Result<Option<IgnoreReason>, String>,
    },
    IgnoreEdited {
        request: u64,
        result: Result<String, String>,
    },
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
    RemotesListed {
        request: u64,
        result: Result<Vec<Remote>, String>,
    },
    RemoteDone {
        request: u64,
        result: Result<BranchDone, String>,
    },
    RemoteImpacted {
        request: u64,
        name: String,
        result: Result<RemoteImpact, String>,
    },
    RemoteChecked {
        request: u64,
        name: String,
        result: Result<RemoteCheck, NetError>,
    },
    FetchProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    FetchDone {
        request: u64,
        result: Result<FetchOutcome, NetError>,
    },
    OperationProbed {
        request: u64,
        kind: Option<OperationKind>,
    },
    ConflictsLoaded {
        request: u64,
        result: Result<Box<ConflictSnapshot>, String>,
    },
    ConflictVersionsLoaded {
        request: u64,
        result: Result<Box<ConflictVersions>, String>,
    },
    ConflictFinished {
        request: u64,
        result: Result<ConflictDone, String>,
    },
    MergePreviewed {
        request: u64,
        result: Result<Box<MergePreview>, String>,
    },
    MergeFinished {
        request: u64,
        result: Result<MergeOutcome, String>,
    },
    StashesListed {
        request: u64,
        result: Result<Vec<Stash>, String>,
    },
    StashDetailed {
        request: u64,
        oid: String,
        result: Result<StashDetail, String>,
    },
    StashFinished {
        request: u64,
        result: Result<StashDone, String>,
    },
    PublishPrepared {
        request: u64,
        remotes: Vec<String>,
        preview: Result<Box<PushPreview>, String>,
    },
    PushPreviewed {
        request: u64,
        result: Result<Box<PushPreview>, String>,
    },
    PushProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    PushFinished {
        request: u64,
        result: Result<PushOutcome, String>,
    },
    ReflogListed {
        request: u64,
        result: Result<Vec<ReflogEntry>, String>,
    },
    ReflogInspected {
        request: u64,
        result: Result<ReflogInspect, String>,
    },
    ReflogRecovered {
        request: u64,
        result: Result<BranchDone, String>,
    },
    ResetPreviewed {
        request: u64,
        result: Result<Box<ResetPreview>, String>,
    },
    ResetFinished {
        request: u64,
        result: Result<ResetOutcome, String>,
    },
    SequencePreviewed {
        request: u64,
        result: Result<Box<SequencePreview>, String>,
    },
    SequenceFinished {
        request: u64,
        result: Result<SequenceOutcome, String>,
    },
    UndoFinished {
        request: u64,
        id: u64,
        direction: UndoDirection,
        result: Result<(), String>,
    },
    RebasePlanned {
        request: u64,
        result: Result<Box<RebasePlanInfo>, String>,
    },
    RebaseFinished {
        request: u64,
        result: Result<RebaseOutcome, String>,
    },
    PullPreviewed {
        request: u64,
        result: Result<Box<PullPreview>, String>,
    },
    PullProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    PullFinished {
        request: u64,
        result: Result<PullOutcome, String>,
    },
    WorktreesListed {
        request: u64,
        result: Result<Vec<Worktree>, String>,
    },
    WorktreeRemovalPreviewed {
        request: u64,
        result: Result<WorktreeRemoval, String>,
    },
    WorktreeFinished {
        request: u64,
        result: Result<BranchDone, String>,
    },
    HooksListed {
        request: u64,
        result: Result<Box<HooksView>, String>,
    },
    HookChanged {
        request: u64,
        name: String,
        enabled: bool,
        result: Result<Box<HooksView>, HookError>,
    },
    LfsLoaded {
        request: u64,
        result: Result<Box<(LfsStatus, Vec<LfsFile>)>, String>,
    },
    LfsProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    LfsFinished {
        request: u64,
        result: Result<(), String>,
    },
    AccountConnected {
        request: u64,
        result: Result<(Connection, AccountSession), String>,
    },
    AccountDeviceCode {
        request: u64,
        code: DeviceCode,
    },
    AccountChecked {
        id: String,
        result: Result<AccountSession, String>,
    },
    GitflowLoaded {
        request: u64,
        result: Result<Box<GitflowView>, String>,
    },
    GitflowPlanned {
        request: u64,
        result: Result<Box<FlowPlan>, String>,
    },
    GitflowFinished {
        request: u64,
        result: Result<FlowOutcome, String>,
    },
    GitflowSaved {
        request: u64,
        result: Result<Vec<String>, String>,
    },
    SubmodulesListed {
        request: u64,
        result: Result<(PathBuf, Vec<Submodule>), String>,
    },
    SubmoduleProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    SubmoduleFinished {
        request: u64,
        result: Result<String, SubmoduleError>,
    },
    CloneProgress {
        request: u64,
        phase: String,
        percent: u8,
    },
    CloneDone {
        request: u64,
        result: Result<PathBuf, CloneError>,
    },
    SshScanned {
        request: u64,
        result: Result<Vec<HostKey>, NetError>,
    },
    SshTrusted {
        request: u64,
        result: Result<(), NetError>,
    },
    InitPrepared {
        request: u64,
        branch: String,
    },
    InitDone {
        request: u64,
        path: PathBuf,
        completed_existing: bool,
    },
    InitFailed {
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
    pub fn start(events: Sender<Event>, waker: WakerSlot, git: Git) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("gitpoulpe-engine")
            .enable_all()
            .build()?;
        let (commands, receiver) = unbounded_channel();
        std::thread::Builder::new()
            .name("gitpoulpe-engine-main".to_owned())
            .spawn(move || {
                let acknowledge = runtime.block_on(run(receiver, events, waker, git));
                runtime.shutdown_timeout(SHUTDOWN_WAIT);
                if let Some(acknowledge) = acknowledge {
                    let _ = acknowledge.send(());
                }
            })?;
        Ok(Self { commands })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn shutdown(&self) {
        let (acknowledge, done) = std::sync::mpsc::channel();
        if self.commands.send(Command::Shutdown(acknowledge)).is_ok() {
            let _ = done.recv_timeout(SHUTDOWN_WAIT + Duration::from_secs(1));
        }
    }
}

struct Session {
    info: SessionInfo,
    generation: Mutex<Generation>,
    writer: tokio::sync::Mutex<()>,
    commit: Mutex<Option<CommitRange>>,
}

impl Session {
    fn new(info: SessionInfo, generation: Generation) -> Self {
        Self {
            info,
            generation: Mutex::new(generation),
            writer: tokio::sync::Mutex::new(()),
            commit: Mutex::new(None),
        }
    }

    fn generation(&self) -> Generation {
        *self.generation.lock().unwrap()
    }

    fn bump(&self) -> Generation {
        let mut generation = self.generation.lock().unwrap();
        *generation = generation.next();
        *generation
    }
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

async fn run(
    mut receiver: UnboundedReceiver<Command>,
    events: Sender<Event>,
    waker: WakerSlot,
    git: Git,
) -> Option<std::sync::mpsc::Sender<()>> {
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
            return None;
        }
    };
    let shared = Arc::new(Shared {
        reader: DiffReader::new(git.clone(), cache),
        git,
        events,
        waker,
    });
    let mut session: Option<Arc<Session>> = None;
    let mut history: Option<HistoryHandle> = None;
    let mut search: Option<HistoryHandle> = None;
    let mut watcher: Option<RepoWatcher> = None;
    let mut net_cancel: Option<Cancel> = None;
    let mut signals: Option<UnboundedReceiver<Signal>> = None;
    let watched = Arc::new(std::sync::Mutex::new(HashSet::new()));
    let mut shutdown = None;
    let (started_sender, mut started_receiver) = unbounded_channel::<Started>();
    loop {
        let signal = async {
            match signals.as_mut() {
                Some(receiver) => receiver.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            command = receiver.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Open { request, path } => {
                        history = None;
                        search = None;
                        watcher = None;
                        signals = None;
                        if let Some(previous) = session.take() {
                            shared.reader.close_session(previous.info.id);
                            watched.lock().unwrap().clear();
                        }
                        match open(&shared, &path).await {
                            Ok((info, snapshot)) => {
                                session = Some(Arc::new(Session::new(info.clone(), snapshot.generation)));
                                let id = info.id;
                                shared.emit(Event::Opened {
                                    request,
                                    info: Box::new(info.clone()),
                                    snapshot: Box::new(snapshot),
                                });
                                start_watching(info, id, started_sender.clone());
                            }
                            Err(message) => shared.emit(Event::OpenFailed { request, message }),
                        }
                    }
                    Command::Refresh { request } => {
                        if let Some(current) = &session {
                            start_refresh(&shared, current, request, false);
                        }
                    }
                    Command::Commit { request, job } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(commit(shared.clone(), current, request, *job));
                    }
                    Command::HistoryOpen { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let handle = HistoryHandle::default();
                        history = Some(handle.clone());
                        tokio::spawn(history_open(shared.clone(), current, request, handle));
                    }
                    Command::HistoryMore { request } => {
                        if let Some(handle) = history.clone() {
                            tokio::spawn(history_page(shared.clone(), request, handle));
                        }
                    }
                    Command::HistoryClose => {
                        history = None;
                        search = None;
                    }
                    Command::SearchOpen { request, text } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let handle = HistoryHandle::default();
                        search = Some(handle.clone());
                        tokio::spawn(search_open(shared.clone(), current, request, text, handle));
                    }
                    Command::SearchMore { request } => {
                        if let Some(handle) = search.clone() {
                            tokio::spawn(search_page(shared.clone(), request, handle));
                        }
                    }
                    Command::SearchClose => search = None,
                    Command::CommitInspect {
                        request,
                        oid,
                        parent_index,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(inspect_commit(
                            shared.clone(),
                            current,
                            request,
                            oid,
                            parent_index,
                        ));
                    }
                    Command::CommitLeave => {
                        if let Some(current) = &session {
                            *current.commit.lock().unwrap() = None;
                        }
                    }
                    Command::Amend { request, job } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(amend(shared.clone(), current, request, *job));
                    }
                    Command::AmendInfo { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let info = shared.git.amend_info(&current.info).await;
                            shared.emit(Event::AmendInfo { request, info });
                        });
                    }
                    Command::RemoteList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_remotes(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::RemotesListed { request, result });
                        });
                    }
                    Command::RemoteOp { request, op } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(remote_op(shared.clone(), current, request, op));
                    }
                    Command::RemoteImpact { request, name } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .remote_impact(&current.info, &name)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::RemoteImpacted {
                                request,
                                name,
                                result,
                            });
                        });
                    }
                    Command::RemoteCheck { request, name } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .check_remote(&current.info, &name, &cancel)
                                .await;
                            shared.emit(Event::RemoteChecked {
                                request,
                                name,
                                result,
                            });
                        });
                    }
                    Command::SshScan {
                        request,
                        host,
                        port,
                    } => {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared.git.scan_ssh_host(&host, port).await;
                            shared.emit(Event::SshScanned { request, result });
                        });
                    }
                    Command::SshTrust {
                        request,
                        host,
                        port,
                        fingerprint,
                    } => {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared.git.trust_ssh_host(&host, port, &fingerprint).await;
                            shared.emit(Event::SshTrusted { request, result });
                        });
                    }
                    Command::Fetch { request, remote } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let mut last = (String::new(), 101u8);
                            let result = shared
                                .git
                                .fetch_remote(&current.info, &remote, &cancel, |progress| {
                                    let key = (progress.phase.clone(), progress.percent);
                                    if key != last {
                                        shared.emit(Event::FetchProgress {
                                            request,
                                            phase: progress.phase,
                                            percent: progress.percent,
                                        });
                                        last = key;
                                    }
                                })
                                .await;
                            shared.emit(Event::FetchDone { request, result });
                        });
                    }
                    Command::Clone {
                        request,
                        url,
                        parent,
                        name,
                    } => {
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let mut last = (String::new(), 101u8);
                            let result = shared
                                .git
                                .clone_repository(&url, &parent, &name, &cancel, |progress| {
                                    let key = (progress.phase.clone(), progress.percent);
                                    if key != last {
                                        shared.emit(Event::CloneProgress {
                                            request,
                                            phase: progress.phase,
                                            percent: progress.percent,
                                        });
                                        last = key;
                                    }
                                })
                                .await;
                            shared.emit(Event::CloneDone { request, result });
                        });
                    }
                    Command::OperationProbe { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let kind = shared
                                .git
                                .operation_state(&current.info)
                                .await
                                .ok()
                                .flatten()
                                .map(|o| o.kind);
                            shared.emit(Event::OperationProbed { request, kind });
                        });
                    }
                    Command::ConflictLoad {
                        request,
                        remembered,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = async {
                                let operation = shared.git.operation_state(&current.info).await?;
                                let files = shared.git.list_conflicts(&current.info).await?;
                                let states = if operation.is_some() {
                                    shared
                                        .git
                                        .resolution_states(&current.info, &remembered)
                                        .await?
                                } else {
                                    Vec::new()
                                };
                                Ok::<_, ConflictError>(Box::new(ConflictSnapshot {
                                    operation,
                                    files,
                                    states,
                                }))
                            }
                            .await
                            .map_err(|e| e.to_string());
                            shared.emit(Event::ConflictsLoaded { request, result });
                        });
                    }
                    Command::ConflictVersions { request, path } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .conflict_versions(&current.info, &path)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ConflictVersionsLoaded { request, result });
                        });
                    }
                    Command::ConflictOp {
                        request,
                        op,
                        remembered,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(conflict_op(shared.clone(), current, request, op, remembered));
                    }
                    Command::MergePreview { request, source } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .merge_preview(&current.info, &source)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::MergePreviewed { request, result });
                        });
                    }
                    Command::Merge {
                        request,
                        source,
                        mode,
                        expected_head,
                        autostash,
                        profile,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .merge_branch(
                                    &current.info,
                                    &source,
                                    mode,
                                    &expected_head,
                                    autostash,
                                    profile.as_deref(),
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::MergeFinished { request, result });
                        });
                    }
                    Command::StashList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_stashes(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::StashesListed { request, result });
                        });
                    }
                    Command::StashDetail { request, oid } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .stash_detail(&current.info, &oid)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::StashDetailed {
                                request,
                                oid,
                                result,
                            });
                        });
                    }
                    Command::StashOp { request, op } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(stash_op(shared.clone(), current, request, op));
                    }
                    Command::PublishOpen { request, branch } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let remotes: Vec<String> = shared
                                .git
                                .list_remotes(&current.info)
                                .await
                                .map(|all| all.into_iter().map(|r| r.name).collect())
                                .unwrap_or_default();
                            let chosen = remotes
                                .iter()
                                .find(|r| r.as_str() == "origin")
                                .or_else(|| remotes.first())
                                .cloned();
                            let preview = match chosen {
                                Some(remote) => shared
                                    .git
                                    .push_preview(&current.info, &branch, &remote)
                                    .await
                                    .map(Box::new)
                                    .map_err(|e| e.to_string()),
                                None => Err(
                                    "Aucun remote n'est configuré : ajoutez-en un depuis l'écran Remotes."
                                        .to_owned(),
                                ),
                            };
                            shared.emit(Event::PublishPrepared {
                                request,
                                remotes,
                                preview,
                            });
                        });
                    }
                    Command::PushPreview {
                        request,
                        branch,
                        remote,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .push_preview(&current.info, &branch, &remote)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::PushPreviewed { request, result });
                        });
                    }
                    Command::Push {
                        request,
                        branch,
                        remote,
                        mode,
                        expected,
                        set_upstream,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let mut last = (String::new(), 101u8);
                            let result = shared
                                .git
                                .push_branch(
                                    &current.info,
                                    &PushRequest {
                                        branch: &branch,
                                        remote: &remote,
                                        mode,
                                        expected_remote_oid: expected.as_deref(),
                                        set_upstream,
                                    },
                                    &cancel,
                                    |progress| {
                                        let key = (progress.phase.clone(), progress.percent);
                                        if key != last {
                                            shared.emit(Event::PushProgress {
                                                request,
                                                phase: progress.phase,
                                                percent: progress.percent,
                                            });
                                            last = key;
                                        }
                                    },
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::PushFinished { request, result });
                        });
                    }
                    Command::ReflogList { request, reference } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_reflog(&current.info, &reference)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ReflogListed { request, result });
                        });
                    }
                    Command::ReflogInspect { request, oid } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .inspect_reflog_entry(&current.info, &oid)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ReflogInspected { request, result });
                        });
                    }
                    Command::ReflogRecover { request, oid, name } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .create_recovery_branch(&current.info, &oid, &name)
                                .await
                                .map(|()| BranchDone {
                                    message: format!(
                                        "Branche « {name} » créée sur {} : la branche courante n'a pas bougé.",
                                        short(&oid)
                                    ),
                                    journal: Some((
                                        "recover-branch".into(),
                                        None,
                                        Some(oid.clone()),
                                        name.clone(),
                                    )),
                                })
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ReflogRecovered { request, result });
                        });
                    }
                    Command::ResetPreview {
                        request,
                        target,
                        mode,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .reset_preview(&current.info, &target, mode)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ResetPreviewed { request, result });
                        });
                    }
                    Command::Reset {
                        request,
                        target,
                        mode,
                        expected_head,
                        confirmed,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .reset_branch(&current.info, &target, mode, &expected_head, confirmed)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::ResetFinished { request, result });
                        });
                    }
                    Command::SequencePreview {
                        request,
                        kind,
                        oids,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .sequence_preview(&current.info, kind, &oids)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::SequencePreviewed { request, result });
                        });
                    }
                    Command::Sequence {
                        request,
                        kind,
                        oids,
                        mainline,
                        empty,
                        expected_head,
                        profile,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .run_sequence(
                                    &current.info,
                                    &SequenceRequest {
                                        kind,
                                        oids: &oids,
                                        mainline,
                                        empty,
                                        expected_head: &expected_head,
                                    },
                                    profile.as_deref(),
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::SequenceFinished { request, result });
                        });
                    }
                    Command::UndoRedo {
                        request,
                        record,
                        direction,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .undo_redo(&current.info, &record, direction)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::UndoFinished {
                                request,
                                id: record.id,
                                direction,
                                result,
                            });
                        });
                    }
                    Command::RebasePlan { request, base } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .rebase_plan(&current.info, &base)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::RebasePlanned { request, result });
                        });
                    }
                    Command::Rebase {
                        request,
                        base,
                        items,
                        expected_head,
                        autostash,
                        profile,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .start_rebase(
                                    &current.info,
                                    &base,
                                    &items,
                                    &expected_head,
                                    autostash,
                                    profile.as_deref(),
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::RebaseFinished { request, result });
                        });
                    }
                    Command::PullPreview { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .pull_preview(&current.info)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::PullPreviewed { request, result });
                        });
                    }
                    Command::Pull {
                        request,
                        strategy,
                        expected_head,
                        autostash,
                        profile,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let mut last = (String::new(), 101u8);
                            let result = shared
                                .git
                                .pull(
                                    &current.info,
                                    &PullRequest {
                                        strategy,
                                        expected_head: &expected_head,
                                        autostash,
                                    },
                                    profile.as_deref(),
                                    &cancel,
                                    |progress| {
                                        let key = (progress.phase.clone(), progress.percent);
                                        if key != last {
                                            shared.emit(Event::PullProgress {
                                                request,
                                                phase: progress.phase,
                                                percent: progress.percent,
                                            });
                                            last = key;
                                        }
                                    },
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::PullFinished { request, result });
                        });
                    }
                    Command::WorktreeList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_worktrees(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::WorktreesListed { request, result });
                        });
                    }
                    Command::WorktreeRemovalPreview { request, path } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .worktree_removal(&current.info, &path)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::WorktreeRemovalPreviewed { request, result });
                        });
                    }
                    Command::WorktreeOp { request, op } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(worktree_op(shared.clone(), current, request, op));
                    }
                    Command::HookList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_hooks(&current.info)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::HooksListed { request, result });
                        });
                    }
                    Command::HookSet {
                        request,
                        name,
                        enabled,
                        confirm_shared,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .set_hook_enabled(&current.info, &name, enabled, confirm_shared)
                                .await
                                .map(Box::new);
                            shared.emit(Event::HookChanged {
                                request,
                                name,
                                enabled,
                                result,
                            });
                        });
                    }
                    Command::LfsLoad { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = async {
                                let status = shared.git.lfs_status(&current.info).await?;
                                let files = shared.git.list_lfs_files(&current.info).await?;
                                Ok::<_, gitpoulpe_git::LfsError>(Box::new((status, files)))
                            }
                            .await
                            .map_err(|e| e.to_string());
                            shared.emit(Event::LfsLoaded { request, result });
                        });
                    }
                    Command::LfsRun {
                        request,
                        kind,
                        branch,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let remote = shared
                                .git
                                .list_remotes(&current.info)
                                .await
                                .ok()
                                .and_then(|all| {
                                    all.iter()
                                        .find(|r| r.name == "origin")
                                        .or_else(|| all.first())
                                        .map(|r| r.name.clone())
                                })
                                .unwrap_or_default();
                            let mut last = (String::new(), 101u8);
                            let result = shared
                                .git
                                .lfs_transfer(
                                    &current.info,
                                    kind,
                                    &remote,
                                    &branch,
                                    &cancel,
                                    |progress| {
                                        let key = (progress.phase.clone(), progress.percent);
                                        if key != last {
                                            shared.emit(Event::LfsProgress {
                                                request,
                                                phase: progress.phase,
                                                percent: progress.percent,
                                            });
                                            last = key;
                                        }
                                    },
                                )
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::LfsFinished { request, result });
                        });
                    }
                    Command::GitflowLoad { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .gitflow_view(&current.info)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::GitflowLoaded { request, result });
                        });
                    }
                    Command::GitflowPlan {
                        request,
                        kind,
                        finish,
                        name,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .gitflow_plan(&current.info, kind, finish, &name)
                                .await
                                .map(Box::new)
                                .map_err(|e| e.to_string());
                            shared.emit(Event::GitflowPlanned { request, result });
                        });
                    }
                    Command::GitflowRun {
                        request,
                        plan,
                        finish,
                        profile,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .gitflow_run(&current.info, &plan, finish, profile.as_deref())
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::GitflowFinished { request, result });
                        });
                    }
                    Command::GitflowSave {
                        request,
                        config,
                        create_develop,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let result = shared
                                .git
                                .gitflow_save(&current.info, &config, create_develop)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::GitflowSaved { request, result });
                        });
                    }
                    Command::SubmoduleList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let root = current
                                .info
                                .worktree
                                .clone()
                                .unwrap_or_else(|| current.info.git_dir.clone());
                            let result = shared
                                .git
                                .list_submodules(&current.info)
                                .await
                                .map(|list| (root, list))
                                .map_err(|e| e.to_string());
                            shared.emit(Event::SubmodulesListed { request, result });
                        });
                    }
                    Command::SubmoduleRun {
                        request,
                        path,
                        init_only,
                        confirm,
                    } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let cancel = Cancel::new();
                        net_cancel = Some(cancel.clone());
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let _guard = current.writer.lock().await;
                            let mut last = (String::new(), 101u8);
                            let result = if init_only {
                                shared
                                    .git
                                    .init_submodule(&current.info, &path)
                                    .await
                                    .map(|()| format!("{} initialisé.", path.display()))
                            } else {
                                shared
                                    .git
                                    .update_submodule(&current.info, &path, confirm, &cancel, |p| {
                                        let key = (p.phase.clone(), p.percent);
                                        if key != last {
                                            shared.emit(Event::SubmoduleProgress {
                                                request,
                                                phase: p.phase,
                                                percent: p.percent,
                                            });
                                            last = key;
                                        }
                                    })
                                    .await
                                    .map(|u| {
                                        let mut message = format!(
                                            "{} est sur le commit attendu {}.",
                                            u.path.display(),
                                            u.to.chars().take(10).collect::<String>()
                                        );
                                        if let Some(marker) = u.marker {
                                            message.push_str(&format!(
                                                " Les commits locaux sont gardés sous {marker} dans le submodule."
                                            ));
                                        }
                                        message
                                    })
                            };
                            shared.emit(Event::SubmoduleFinished { request, result });
                        });
                    }
                    Command::NetCancel => {
                        if let Some(cancel) = net_cancel.take() {
                            cancel.cancel();
                        }
                    }
                    Command::InitPrepare { request, path } => {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let branch = shared.git.default_branch_name(&path).await;
                            shared.emit(Event::InitPrepared { request, branch });
                        });
                    }
                    Command::Init {
                        request,
                        path,
                        branch,
                    } => {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            match shared
                                .git
                                .init_repository(&path, &branch, &Cancel::new())
                                .await
                            {
                                Ok(done) => shared.emit(Event::InitDone {
                                    request,
                                    path,
                                    completed_existing: done.completed_existing,
                                }),
                                Err(error) => shared.emit(Event::InitFailed {
                                    request,
                                    message: error.to_string(),
                                }),
                            }
                        });
                    }
                    Command::TagList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_tags(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::TagsListed { request, result });
                        });
                    }
                    Command::TagOp { request, op } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(tag_op(shared.clone(), current, request, op));
                    }
                    Command::BranchList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_branches(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::BranchesListed { request, result });
                        });
                    }
                    Command::BranchOp { request, op } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(branch_op(shared.clone(), current, request, op));
                    }
                    Command::IgnoreList { request } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .list_ignored(&current.info)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::IgnoreListed { request, result });
                        });
                    }
                    Command::IgnoreReason { request, path } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let result = shared
                                .git
                                .ignore_reason(&current.info, &path)
                                .await
                                .map_err(|e| e.to_string());
                            shared.emit(Event::IgnoreReasoned {
                                request,
                                path,
                                result,
                            });
                        });
                    }
                    Command::IgnoreEdit { request, edit } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(ignore_edit(shared.clone(), current, request, edit));
                    }
                    Command::Identity { request, profile } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(identity(shared.clone(), current, request, profile));
                    }
                    Command::CheckSigning { request, profile } => {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            let check = check_signing(&profile).await;
                            shared.emit(Event::Signing { request, check });
                        });
                    }
                    Command::Write { request, job } => {
                        let Some(current) = session.clone() else {
                            continue;
                        };
                        tokio::spawn(write(shared.clone(), current, request, *job));
                    }
                    Command::Copy(request) => {
                        let Some(current) = session
                            .clone()
                            .filter(|s| s.generation() == request.generation)
                        else {
                            continue;
                        };
                        tokio::spawn(copy(shared.clone(), current, *request));
                    }
                    Command::Read { generation, side, index, entry, range } => {
                        let Some(current) = session.clone().filter(|s| s.generation() == generation) else {
                            continue;
                        };
                        tokio::spawn(read(shared.clone(), current, watched.clone(), side, index, *entry, range));
                    }
                    Command::Shutdown(ack) => {
                        shutdown = Some(ack);
                        break;
                    }
                }
            }
            started = started_receiver.recv() => {
                let Some(started) = started else { continue };
                if session.as_ref().is_some_and(|s| s.info.id == started.id) {
                    shared.emit(started.state);
                    (watcher, signals) = (started.watcher, started.signals);
                }
            },
            signal = signal => match signal {
                Some(signal) => {
                    if let Some(current) = &session {
                        start_refresh(&shared, current, 0, signal == Signal::EventsLost);
                    }
                }
                None => signals = None,
            },
        }
    }
    drop(watcher);
    drop(signals);
    if let Some(current) = session.take() {
        shared.reader.close_session(current.info.id);
    }
    shutdown
}

struct Started {
    id: SessionId,
    state: Event,
    watcher: Option<RepoWatcher>,
    signals: Option<UnboundedReceiver<Signal>>,
}

fn start_watching(info: SessionInfo, id: SessionId, started: UnboundedSender<Started>) {
    tokio::task::spawn_blocking(move || {
        let (raw_sender, raw_receiver) = unbounded_channel();
        let (out_sender, out_receiver) = unbounded_channel();
        let result = match RepoWatcher::start(&info, raw_sender) {
            Ok(watcher) => {
                tokio::spawn(coalesce(raw_receiver, out_sender, QUIET_PERIOD, MAX_WAIT));
                Started {
                    id,
                    state: Event::WatchState {
                        available: true,
                        reason: String::new(),
                    },
                    watcher: Some(watcher),
                    signals: Some(out_receiver),
                }
            }
            Err(error) => Started {
                id,
                state: Event::WatchState {
                    available: false,
                    reason: error.to_string(),
                },
                watcher: None,
                signals: None,
            },
        };
        let _ = started.send(result);
    });
}

fn start_refresh(shared: &Arc<Shared>, session: &Arc<Session>, request: u64, events_lost: bool) {
    let generation = session.bump();
    shared.reader.retire_before(session.info.id, generation);
    let (shared, session) = (shared.clone(), session.clone());
    tokio::spawn(async move {
        let result = shared
            .git
            .read_status(&session.info, generation, &Cancel::new())
            .await;
        if session.generation() != generation {
            return;
        }
        match result {
            Ok(snapshot) => shared.emit(Event::Refreshed {
                request,
                snapshot: Box::new(snapshot),
                events_lost,
            }),
            Err(error) => shared.emit(Event::RefreshFailed {
                request,
                message: describe_read_error(error),
            }),
        }
    });
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
        .map_err(describe_read_error)?;
    Ok((info, snapshot))
}

fn short(oid: &str) -> String {
    oid.chars().take(8).collect()
}

async fn remote_op(shared: Arc<Shared>, session: Arc<Session>, request: u64, op: RemoteOp) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let result = match &op {
        RemoteOp::Add { name, url } => {
            git.add_remote(info, name, url)
                .await
                .map(|remote| BranchDone {
                    message: format!("Remote {} ajouté.", remote.name),
                    journal: Some((
                        "remote-add".into(),
                        None,
                        remote.fetch_urls.first().map(|u| mask_url(u)),
                        remote.name,
                    )),
                })
        }
        RemoteOp::SetUrl {
            name,
            url,
            expected,
        } => git
            .set_remote_url(info, name, url, expected)
            .await
            .map(|remote| BranchDone {
                message: format!("URL de lecture de {} modifiée.", remote.name),
                journal: Some((
                    "remote-set-url".into(),
                    expected.first().map(|u| mask_url(u)),
                    remote.fetch_urls.first().map(|u| mask_url(u)),
                    remote.name,
                )),
            }),
        RemoteOp::SetPushUrl {
            name,
            url,
            expected,
        } => git
            .set_remote_push_url(info, name, url.as_deref(), expected)
            .await
            .map(|remote| BranchDone {
                message: if remote.push_urls.is_empty() {
                    format!("{} envoie désormais vers son URL de lecture.", remote.name)
                } else {
                    format!("URL d'envoi de {} modifiée.", remote.name)
                },
                journal: Some((
                    "remote-set-push-url".into(),
                    expected.first().map(|u| mask_url(u)),
                    remote.push_urls.first().map(|u| mask_url(u)),
                    remote.name,
                )),
            }),
        RemoteOp::Rename { old, new } => {
            git.rename_remote(info, old, new)
                .await
                .map(|remote| BranchDone {
                    message: format!(
                        "Remote {old} renommé en {}. Ses branches de suivi ont suivi.",
                        remote.name
                    ),
                    journal: Some((
                        "remote-rename".into(),
                        Some(old.clone()),
                        Some(remote.name),
                        old.clone(),
                    )),
                })
        }
        RemoteOp::Remove { name, expected } => {
            git.remove_remote(info, name, expected)
                .await
                .map(|removed| {
                    let url = removed.fetch_urls.first().cloned().unwrap_or_default();
                    BranchDone {
                        message: format!(
                            "Remote {name} supprimé. Pour le rétablir : {}",
                            crate::shell::restore_remote(name, &url)
                        ),
                        journal: Some((
                            "remote-remove".into(),
                            Some(mask_url(&url)),
                            None,
                            name.clone(),
                        )),
                    }
                })
        }
    };
    shared.emit(Event::RemoteDone {
        request,
        result: result.map_err(|e| e.to_string()),
    });
}

async fn conflict_op(
    shared: Arc<Shared>,
    session: Arc<Session>,
    request: u64,
    op: ConflictOp,
    remembered: Vec<PathBuf>,
) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let done = |message: String| ConflictDone {
        message,
        finished: false,
    };
    let result: Result<ConflictDone, ConflictError> = match &op {
        ConflictOp::Side { path, side } => git
            .resolve_with_side(info, path, *side)
            .await
            .map(|()| done(format!("{} résolu.", path.display()))),
        ConflictOp::Content {
            path,
            content,
            allow_markers,
        } => git
            .resolve_with_content(info, path, content.as_bytes(), *allow_markers)
            .await
            .map(|()| done(format!("{} résolu.", path.display()))),
        ConflictOp::Revalidate { path } => git
            .revalidate_resolved(info, path)
            .await
            .map(|()| done(format!("{} validé de nouveau.", path.display()))),
        ConflictOp::Continue { profile } => git
            .continue_operation(info, profile.as_deref(), &remembered)
            .await
            .map(|o| ConflictDone {
                message: o.message,
                finished: o.finished,
            }),
        ConflictOp::Skip => git.skip_operation(info).await.map(|o| ConflictDone {
            message: o.message,
            finished: o.finished,
        }),
        ConflictOp::Abort => git.abort_operation(info, &remembered).await.map(|report| {
            ConflictDone {
                message: match &report.backup {
                    Some(dir) => format!(
                        "Opération abandonnée, l'état d'avant est rétabli. {} fichier(s) de votre travail de résolution sont conservés dans {}.",
                        report.saved,
                        dir.display()
                    ),
                    None => "Opération abandonnée, l'état d'avant est rétabli.".to_owned(),
                },
                finished: true,
            }
        }),
    };
    shared.emit(Event::ConflictFinished {
        request,
        result: result.map_err(|e| match e {
            ConflictError::MarkersRemain => "MARKERS".to_owned(),
            other => other.to_string(),
        }),
    });
}

async fn stash_op(shared: Arc<Shared>, session: Arc<Session>, request: u64, op: StashOp) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let result: Result<StashDone, StashError> = match &op {
        StashOp::Create {
            message,
            scope,
            profile,
        } => git
            .create_stash(info, message.as_deref(), *scope, profile.as_deref())
            .await
            .map(|stash| StashDone {
                message: format!(
                    "Travail mis de côté dans stash@{{0}} ({}).",
                    if stash.message.is_empty() {
                        "sans message".to_owned()
                    } else {
                        stash.message.clone()
                    }
                ),
                conflicts: false,
                journal: Some(("stash-create".into(), None, Some(stash.oid), stash.message)),
            }),
        StashOp::Apply {
            oid,
            pop,
            restore_index,
        } => git
            .apply_stash(info, oid, *pop, *restore_index)
            .await
            .map(|applied| match applied {
                StashApplied::Clean { dropped } => StashDone {
                    message: if dropped {
                        "Stash appliqué puis retiré de la liste.".to_owned()
                    } else {
                        "Stash appliqué ; il reste dans la liste.".to_owned()
                    },
                    conflicts: false,
                    journal: Some((
                        if *pop { "stash-pop" } else { "stash-apply" }.into(),
                        Some(oid.clone()),
                        None,
                        String::new(),
                    )),
                },
                StashApplied::Conflicts { files } => StashDone {
                    message: format!(
                        "L'application a produit des conflits dans {} fichier(s). Le stash est conservé : résolvez les conflits, il ne sera supprimé que par vous.",
                        files.len()
                    ),
                    conflicts: true,
                    journal: Some((
                        "stash-apply-conflict".into(),
                        Some(oid.clone()),
                        None,
                        String::new(),
                    )),
                },
            }),
        StashOp::Drop { oid } => git.drop_stash(info, oid).await.map(|stash| StashDone {
            message: format!(
                "Stash supprimé. Pour le rétablir : {}",
                crate::shell::restore_stash(&stash.message, &stash.oid)
            ),
            conflicts: false,
            journal: Some(("stash-drop".into(), Some(stash.oid), None, stash.message)),
        }),
    };
    shared.emit(Event::StashFinished {
        request,
        result: result.map_err(|e| e.to_string()),
    });
}

async fn worktree_op(shared: Arc<Shared>, session: Arc<Session>, request: u64, op: WorktreeOp) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let done = |message: String,
                journal: Option<(String, Option<String>, Option<String>, String)>| {
        BranchDone { message, journal }
    };
    let result: Result<BranchDone, WorktreeError> = match &op {
        WorktreeOp::Add {
            path,
            branch,
            create,
        } => git
            .add_worktree(
                info,
                path,
                if *create {
                    BranchChoice::New {
                        name: branch,
                        start: None,
                    }
                } else {
                    BranchChoice::Existing(branch)
                },
            )
            .await
            .map(|()| {
                done(
                    format!("Worktree créé dans {} sur la branche {branch}.", path.display()),
                    Some(("worktree-add".into(), None, None, path.display().to_string())),
                )
            }),
        WorktreeOp::Lock { path, reason } => git
            .lock_worktree(info, path, reason)
            .await
            .map(|()| done(format!("{} verrouillé.", path.display()), None)),
        WorktreeOp::Unlock { path } => git
            .unlock_worktree(info, path)
            .await
            .map(|()| done(format!("{} déverrouillé.", path.display()), None)),
        WorktreeOp::Remove { path, discard } => {
            git.remove_worktree(info, path, *discard)
                .await
                .map(|removed| {
                    let mut message = format!("Worktree {} supprimé.", removed.path.display());
                    if let Some(snapshot) = &removed.snapshot {
                        message.push_str(&format!(
                            " Les modifications suivies sont sauvegardées sous {snapshot} (git stash apply <id>)."
                        ));
                    }
                    if removed.lost_untracked > 0 {
                        message.push_str(&format!(
                            " {} fichier(s) non suivi(s) ont été effacés et ne sont pas récupérables.",
                            removed.lost_untracked
                        ));
                    }
                    done(
                        message,
                        Some(("worktree-remove".into(), None, None, path.display().to_string())),
                    )
                })
        }
        WorktreeOp::Prune => git
            .prune_worktrees(info)
            .await
            .map(|()| done("Worktrees orphelins nettoyés.".to_owned(), None)),
    };
    shared.emit(Event::WorktreeFinished {
        request,
        result: result.map_err(|e| e.to_string()),
    });
}

async fn tag_op(shared: Arc<Shared>, session: Arc<Session>, request: u64, op: TagOp) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let result = match &op {
        TagOp::Create {
            name,
            target,
            message,
            profile,
        } => git
            .create_tag(
                info,
                name,
                target.as_deref(),
                message.as_deref(),
                profile.as_deref(),
            )
            .await
            .map(|tag| BranchDone {
                message: format!(
                    "Tag {} {} créé sur {}.",
                    tag.name,
                    if tag.annotated { "annoté" } else { "léger" },
                    tag.target.as_deref().map(short).unwrap_or_default()
                ),
                journal: Some(("tag-create".into(), None, Some(tag.object), tag.name)),
            }),
        TagOp::Delete { name, expected } => {
            git.delete_tag(info, name, expected)
                .await
                .map(|object| BranchDone {
                    message: format!(
                        "Tag {name} supprimé (aucune branche touchée). Pour le rétablir : {}",
                        crate::shell::restore_tag(name, &object)
                    ),
                    journal: Some(("tag-delete".into(), Some(object), None, name.clone())),
                })
        }
    };
    shared.emit(Event::TagDone {
        request,
        result: result.map_err(|e| e.to_string()),
    });
}

async fn branch_op(shared: Arc<Shared>, session: Arc<Session>, request: u64, op: BranchOp) {
    let _guard = session.writer.lock().await;
    let git = &shared.git;
    let info = &session.info;
    let result: Result<BranchDone, BranchError> = match &op {
        BranchOp::Create { name } => match git.create_branch(info, name, None).await {
            Ok(()) => {
                let tip = git
                    .list_branches(info)
                    .await
                    .ok()
                    .and_then(|all| all.into_iter().find(|b| &b.name == name))
                    .map(|b| b.oid);
                Ok(BranchDone {
                    message: format!("Branche {name} créée à partir de HEAD."),
                    journal: Some(("branch-create".into(), None, tip, name.clone())),
                })
            }
            Err(error) => Err(error),
        },
        BranchOp::Switch { name } => {
            git.switch_branch(info, name)
                .await
                .map(|(before, after)| BranchDone {
                    message: format!("Sur la branche {name}."),
                    journal: Some(("switch".into(), before, Some(after), name.clone())),
                })
        }
        BranchOp::Rename { old, new } => {
            git.rename_branch(info, old, new)
                .await
                .map(|()| BranchDone {
                    message: format!("Branche {old} renommée en {new}."),
                    journal: Some((
                        "branch-rename".into(),
                        Some(old.clone()),
                        Some(new.clone()),
                        format!("{old} -> {new}"),
                    )),
                })
        }
        BranchOp::Delete { name, mode } => match git.delete_branch(info, name, mode).await {
            Ok(tip) => Ok(BranchDone {
                message: format!(
                    "Branche {name} supprimée (elle pointait sur {}). Pour la rétablir : {}",
                    short(&tip),
                    crate::shell::restore_branch(name, &tip)
                ),
                journal: Some(("branch-delete".into(), Some(tip), None, name.clone())),
            }),
            Err(BranchError::NotMerged) => match git.branch_loss(info, name).await {
                Ok(loss) => {
                    shared.emit(Event::BranchNeedsConfirmation {
                        request,
                        name: name.clone(),
                        loss: Box::new(loss),
                    });
                    return;
                }
                Err(other) => Err(other),
            },
            Err(other) => Err(other),
        },
    };
    shared.emit(Event::BranchDone {
        request,
        result: result.map_err(|e| e.to_string()),
    });
}

async fn ignore_edit(shared: Arc<Shared>, session: Arc<Session>, request: u64, edit: IgnoreEdit) {
    let _guard = session.writer.lock().await;
    let result = match &edit {
        IgnoreEdit::Add { scope, rule } => shared
            .git
            .add_ignore_rule(&session.info, scope, rule)
            .await
            .map(|file| format!("Règle {rule} ajoutée à {}.", file.display())),
        IgnoreEdit::Remove { scope, rule } => shared
            .git
            .remove_ignore_rule(&session.info, scope, rule)
            .await
            .map(|file| format!("Règle {rule} retirée de {}.", file.display())),
    }
    .map_err(|e: IgnoreError| e.to_string());
    shared.emit(Event::IgnoreEdited { request, result });
}

async fn identity(
    shared: Arc<Shared>,
    session: Arc<Session>,
    request: u64,
    profile: Option<Box<Profile>>,
) {
    let result = shared
        .git
        .effective_identity(&session.info, profile.as_deref(), &Cancel::new())
        .await
        .map_err(|error| match error {
            IdentityError::Unknown(message) => message,
            other => other.to_string(),
        });
    let signing = match profile.as_deref() {
        Some(profile) => check_signing(profile).await,
        None => SigningCheck::NotRequested,
    };
    shared.emit(Event::Identity {
        request,
        result,
        signing,
    });
}

type HistoryHandle = Arc<tokio::sync::Mutex<Option<HistoryStream>>>;

const HISTORY_PAGE: usize = 200;

async fn history_open(
    shared: Arc<Shared>,
    session: Arc<Session>,
    request: u64,
    handle: HistoryHandle,
) {
    match shared.git.history(&session.info).await {
        Ok(stream) => {
            *handle.lock().await = Some(stream);
            history_page(shared, request, handle).await;
        }
        Err(error) => emit_history_error(&shared, request, error),
    }
}

fn emit_history_error(shared: &Shared, request: u64, error: HistoryError) {
    shared.emit(Event::HistoryFailed {
        request,
        no_commits: error == HistoryError::NoCommits,
        message: error.to_string(),
    });
}

async fn history_page(shared: Arc<Shared>, request: u64, handle: HistoryHandle) {
    let mut guard = handle.lock().await;
    let Some(stream) = guard.as_mut() else {
        return;
    };
    match stream.next_page(HISTORY_PAGE).await {
        Ok(commits) => {
            let done = commits.len() < HISTORY_PAGE || stream.is_exhausted();
            shared.emit(Event::HistoryPage {
                request,
                commits,
                done,
            });
        }
        Err(error) => emit_history_error(&shared, request, error),
    }
}

async fn inspect_commit(
    shared: Arc<Shared>,
    session: Arc<Session>,
    request: u64,
    oid: String,
    parent_index: usize,
) {
    let outcome = async {
        let detail = shared.git.commit_detail(&session.info, &oid).await?;
        let parents = &detail.commit.parents;
        let base_index = parent_index.min(parents.len().saturating_sub(1));
        let base = match parents.get(base_index) {
            Some(parent) => parent.clone(),
            None => shared.git.empty_tree(&session.info).await?,
        };
        let entries = shared
            .git
            .commit_files(&session.info, &detail.commit.oid, &base)
            .await?;
        Ok::<_, InspectError>((detail, base, base_index, entries))
    }
    .await;
    match outcome {
        Ok((detail, base, base_index, entries)) => {
            *session.commit.lock().unwrap() = Some(CommitRange {
                oid: detail.commit.oid.clone(),
                base,
            });
            let generation = session.bump();
            shared.reader.retire_before(session.info.id, generation);
            shared.emit(Event::CommitInspected {
                request,
                generation,
                detail: Box::new(detail),
                base_index,
                entries,
            });
        }
        Err(error) => shared.emit(Event::CommitInspectFailed {
            request,
            message: error.to_string(),
        }),
    }
}

const SEARCH_PAGE: usize = 50;

async fn search_open(
    shared: Arc<Shared>,
    session: Arc<Session>,
    request: u64,
    text: String,
    handle: HistoryHandle,
) {
    match shared.git.search(&session.info, &text).await {
        Ok(stream) => {
            *handle.lock().await = Some(stream);
            search_page(shared, request, handle).await;
        }
        Err(error) => shared.emit(Event::SearchFailed {
            request,
            message: error.to_string(),
        }),
    }
}

async fn search_page(shared: Arc<Shared>, request: u64, handle: HistoryHandle) {
    let mut guard = handle.lock().await;
    let Some(stream) = guard.as_mut() else {
        return;
    };
    match stream.next_page(SEARCH_PAGE).await {
        Ok(commits) => {
            let done = commits.len() < SEARCH_PAGE || stream.is_exhausted();
            shared.emit(Event::SearchPage {
                request,
                commits,
                done,
            });
        }
        Err(error) => shared.emit(Event::SearchFailed {
            request,
            message: error.to_string(),
        }),
    }
}

async fn amend(shared: Arc<Shared>, session: Arc<Session>, request: u64, job: AmendRequest) {
    let result = {
        let _writer = session.writer.lock().await;
        shared.git.amend(&session.info, &job, &Cancel::new()).await
    };
    shared.emit(Event::Committed { request, result });
    start_refresh(&shared, &session, 0, false);
}

async fn commit(shared: Arc<Shared>, session: Arc<Session>, request: u64, job: CommitRequest) {
    let result = {
        let _writer = session.writer.lock().await;
        shared.git.commit(&session.info, &job, &Cancel::new()).await
    };
    shared.emit(Event::Committed { request, result });
    start_refresh(&shared, &session, 0, false);
}

async fn write(shared: Arc<Shared>, session: Arc<Session>, request: u64, job: WriteJob) {
    let result = {
        let _writer = session.writer.lock().await;
        shared.git.write(&session.info, &job, &Cancel::new()).await
    };
    shared.emit(Event::Staged { request, result });
    start_refresh(&shared, &session, 0, false);
}

fn describe_read_error(error: ReadError) -> String {
    match error {
        ReadError::NoWorktree => {
            "Ce dépôt n'a pas de répertoire de travail (dépôt bare) : il n'y a aucune modification à inspecter."
                .to_owned()
        }
        ReadError::Cancelled => "Lecture annulée.".to_owned(),
        ReadError::GitUnavailable(d) => format!("Git est introuvable : {d}"),
        ReadError::GitFailed(d) => format!("Git a échoué : {d}"),
        ReadError::Parse(m) => format!("Réponse de Git inattendue : {m}"),
    }
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
    let generation = session.generation();
    let opened = if side == DiffSide::Commit {
        let range = session.commit.lock().unwrap().clone();
        match range {
            Some(range) => {
                shared
                    .reader
                    .open_commit_document(&session.info, generation, &range, &entry)
            }
            None => Err(DiffError::NotInThisView),
        }
    } else {
        shared
            .reader
            .open_document(&session.info, generation, side, &entry)
    };
    let document = match opened {
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
