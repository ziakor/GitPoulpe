#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub object: String,
    pub target: Option<String>,
    pub annotated: bool,
    pub tagger: Option<String>,
    pub tagger_email: Option<String>,
    pub timestamp: Option<i64>,
    pub message: Option<String>,
    pub subject: String,
}
