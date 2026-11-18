#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflogEntry {
    pub index: usize,
    pub selector: String,
    pub oid: String,
    pub previous_oid: String,
    pub action: String,
    pub message: String,
    pub timestamp: i64,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflogInspect {
    pub oid: String,
    pub available: bool,
    pub subject: String,
    pub author: String,
    pub timestamp: i64,
    pub parents: Vec<String>,
    pub reachable_from: Vec<String>,
}
