#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsPointer {
    pub oid: String,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsFileState {
    Present,
    PointerOnly,
    Missing,
    Modified,
    NotConverted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsFile {
    pub path: std::path::PathBuf,
    pub pointer: Option<LfsPointer>,
    pub state: LfsFileState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub detail: String,
    pub uses_lfs: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfsTransfer {
    Fetch,
    Pull,
    Push,
}

impl LfsTransfer {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Fetch => "récupération des objets LFS",
            Self::Pull => "téléchargement des objets LFS",
            Self::Push => "envoi des objets LFS",
        }
    }
}
