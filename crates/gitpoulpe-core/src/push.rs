#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushKind {
    NewBranch,
    FastForward,
    UpToDate,
    Diverged,
    Behind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostRemoteCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPreview {
    pub branch: String,
    pub remote: String,
    pub destination: String,
    pub local_oid: String,
    pub known_remote_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub kind: PushKind,
    pub would_discard: Vec<LostRemoteCommit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushMode {
    Normal,
    ForceWithLease,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOutcome {
    pub remote: String,
    pub destination: String,
    pub pushed_oid: String,
    pub verified: bool,
    pub up_to_date: bool,
    pub set_upstream: bool,
}
