use std::time::Duration;

use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: u64 = 64 * 1024 * 1024;
const MAX_PAGES: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    InvalidUrl,
    Insecure,
    Offline(String),
    Timeout,
    Tls(String),
    TooLarge,
    Other(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl => f.write_str("adresse du serveur invalide"),
            Self::Insecure => {
                f.write_str("le serveur doit être en HTTPS : un jeton n'est jamais envoyé en clair")
            }
            Self::Offline(why) => write!(f, "serveur injoignable : {why}"),
            Self::Timeout => f.write_str("le serveur n'a pas répondu à temps"),
            Self::Tls(why) => write!(f, "connexion sécurisée refusée : {why}"),
            Self::TooLarge => f.write_str("réponse trop volumineuse, abandonnée"),
            Self::Other(why) => write!(f, "erreur réseau : {why}"),
        }
    }
}

impl std::error::Error for HttpError {}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Result<Value, HttpError> {
        serde_json::from_slice(&self.body)
            .map_err(|_| HttpError::Other("réponse illisible".to_owned()))
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub struct Token(String);

impl Token {
    pub fn new(value: &str) -> Self {
        Self(value.trim().to_owned())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(***)")
    }
}

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    authorization: Option<String>,
    accept: &'static str,
    agent: ureq::Agent,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("base", &self.base)
            .field("authenticated", &self.authorization.is_some())
            .finish()
    }
}

fn host_of(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some((scheme, authority))
}

fn is_loopback(authority: &str) -> bool {
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

pub fn validate_base(base: &str) -> Result<String, HttpError> {
    let base = base.trim().trim_end_matches('/');
    let (scheme, authority) = host_of(base).ok_or(HttpError::InvalidUrl)?;
    if base.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(HttpError::InvalidUrl);
    }
    match scheme {
        "https" => {}
        "http" if is_loopback(authority) => {}
        "http" => return Err(HttpError::Insecure),
        _ => return Err(HttpError::InvalidUrl),
    }
    Ok(base.to_owned())
}

fn map_error(error: ureq::Error) -> HttpError {
    match error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            HttpError::Tls("certificat ou protocole invalide".to_owned())
        }
        ureq::Error::Io(e) => HttpError::Offline(e.kind().to_string()),
        ureq::Error::HostNotFound => HttpError::Offline("hôte introuvable".to_owned()),
        ureq::Error::ConnectionFailed => HttpError::Offline("connexion refusée".to_owned()),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => {
            HttpError::Tls("certificat ou protocole invalide".to_owned())
        }
        ureq::Error::BodyExceedsLimit(_) => HttpError::TooLarge,
        other => HttpError::Other(other.to_string()),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(TIMEOUT))
        .user_agent("GitPoulpe")
        .build()
        .into()
}

struct Outgoing {
    method: &'static str,
    url: String,
    accept: &'static str,
    authorization: Option<String>,
    body: Option<Value>,
    limit: u64,
}

async fn run(agent: ureq::Agent, outgoing: Outgoing) -> Result<Response, HttpError> {
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::http::Request::builder()
            .method(outgoing.method)
            .uri(&outgoing.url)
            .header("Accept", outgoing.accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(value) = outgoing.authorization {
            request = request.header("Authorization", value);
        }
        let payload = match outgoing.body {
            Some(value) => {
                request = request.header("Content-Type", "application/json");
                serde_json::to_vec(&value).unwrap_or_default()
            }
            None => Vec::new(),
        };
        let request = request.body(payload).map_err(|_| HttpError::InvalidUrl)?;
        let mut response = agent.run(request).map_err(map_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        let body = response
            .body_mut()
            .with_config()
            .limit(outgoing.limit)
            .read_to_vec()
            .map_err(map_error)?;
        Ok(Response {
            status,
            headers,
            body,
        })
    })
    .await
    .map_err(|e| HttpError::Other(e.to_string()))?
}

pub(crate) fn origin(url: &str) -> Option<String> {
    let (scheme, authority) = host_of(url)?;
    Some(format!("{scheme}://{authority}"))
}

pub(crate) fn authority(url: &str) -> Option<&str> {
    host_of(url).map(|(_, authority)| authority)
}

pub(crate) async fn get_anonymous(url: &str, limit: u64) -> Result<Response, HttpError> {
    validate_base(url)?;
    run(
        agent(),
        Outgoing {
            method: "GET",
            url: url.trim().to_owned(),
            accept: "*/*",
            authorization: None,
            body: None,
            limit,
        },
    )
    .await
}

impl ApiClient {
    pub fn new(base: &str, token: Option<&Token>) -> Result<Self, HttpError> {
        let base = validate_base(base)?;
        Ok(Self {
            base,
            authorization: token.map(|t| format!("Bearer {}", t.expose())),
            accept: "application/vnd.github+json",
            agent: agent(),
        })
    }

    pub fn with_plain_json(mut self) -> Self {
        self.accept = "application/json";
        self
    }

    pub(crate) fn with_accept(mut self, accept: &'static str) -> Self {
        self.accept = accept;
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn same_origin(&self, url: &str) -> bool {
        host_of(url) == host_of(&self.base)
    }

    fn resolve(&self, path_or_url: &str) -> Result<String, HttpError> {
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            if !self.same_origin(path_or_url) {
                return Err(HttpError::Other(
                    "adresse hors du serveur configuré refusée".to_owned(),
                ));
            }
            Ok(path_or_url.to_owned())
        } else {
            Ok(format!(
                "{}/{}",
                self.base,
                path_or_url.trim_start_matches('/')
            ))
        }
    }

    pub async fn send(
        &self,
        method: &'static str,
        path_or_url: &str,
        body: Option<Value>,
    ) -> Result<Response, HttpError> {
        let outgoing = Outgoing {
            method,
            url: self.resolve(path_or_url)?,
            accept: self.accept,
            authorization: self.authorization.clone(),
            body,
            limit: MAX_BODY,
        };
        run(self.agent.clone(), outgoing).await
    }

    pub(crate) async fn get_bytes(
        &self,
        path_or_url: &str,
        accept: &'static str,
        limit: u64,
    ) -> Result<Response, HttpError> {
        let outgoing = Outgoing {
            method: "GET",
            url: self.resolve(path_or_url)?,
            accept,
            authorization: self.authorization.clone(),
            body: None,
            limit,
        };
        run(self.agent.clone(), outgoing).await
    }

    pub async fn get(&self, path: &str) -> Result<Response, HttpError> {
        self.send("GET", path, None).await
    }

    pub async fn get_paged(&self, path: &str, limit_items: usize) -> Result<Vec<Value>, HttpError> {
        let mut items = Vec::new();
        let mut next = Some(path.to_owned());
        let mut pages = 0;
        while let Some(url) = next.take() {
            pages += 1;
            let response = self.get(&url).await?;
            if !response.ok() {
                return Err(HttpError::Other(format!("statut {}", response.status)));
            }
            if let Value::Array(page) = response.json()? {
                items.extend(page);
            }
            if items.len() >= limit_items || pages >= MAX_PAGES {
                break;
            }
            next = response.header("link").and_then(next_link);
        }
        items.truncate(limit_items);
        Ok(items)
    }
}

pub fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
    })
}
