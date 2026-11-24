use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use serde_json::Value;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

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

fn internal_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
        || (a == 192 && b == 0 && c == 0)
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

fn internal_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
    {
        return true;
    }
    if let Some(v4) = ip.to_ipv4() {
        return internal_v4(v4);
    }
    match ip.segments() {
        [0x64, 0xff9b, 0, 0, 0, 0, high, low] => internal_v4(embedded_v4(high, low)),
        [0x2002, high, low, ..] => internal_v4(embedded_v4(high, low)),
        _ => false,
    }
}

pub(crate) fn internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => internal_v4(ip),
        IpAddr::V6(ip) => internal_v6(ip),
    }
}

#[derive(Debug)]
struct InternalAddress(String);

impl std::fmt::Display for InternalAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} désigne une adresse interne : refusé", self.0)
    }
}

impl std::error::Error for InternalAddress {}

#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let found = self.0.resolve(uri, config, timeout)?;
        if found.iter().any(|addr| internal_ip(addr.ip())) {
            let host = uri.host().unwrap_or_default().to_owned();
            return Err(ureq::Error::Io(std::io::Error::other(InternalAddress(
                host,
            ))));
        }
        Ok(found)
    }
}

fn map_error(error: ureq::Error) -> HttpError {
    match error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::Io(e)
            if e.get_ref()
                .is_some_and(|inner| inner.is::<InternalAddress>()) =>
        {
            HttpError::Other(e.to_string())
        }
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

type Builder = ureq::config::ConfigBuilder<ureq::typestate::AgentScope>;

fn base() -> Builder {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(TIMEOUT))
        .user_agent("GitPoulpe")
}

fn agent() -> ureq::Agent {
    base().build().into()
}

fn direct(builder: Builder) -> ureq::config::Config {
    builder.proxy(None).build()
}

fn public_agent() -> ureq::Agent {
    ureq::Agent::with_parts(
        direct(base()),
        DefaultConnector::default(),
        PublicOnly::default(),
    )
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

pub(crate) async fn get_anonymous(
    url: &str,
    limit: u64,
    public_only: bool,
) -> Result<Response, HttpError> {
    validate_base(url)?;
    run(
        if public_only { public_agent() } else { agent() },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_public_only_fetch_refuses_names_and_addresses_that_resolve_inside() {
        for url in [
            "http://localhost:9/x",
            "http://127.0.0.1:9/x",
            "http://[::1]:9/x",
        ] {
            let refused = get_anonymous(url, 1024, true).await.unwrap_err();
            assert!(
                matches!(&refused, HttpError::Other(why) if why.contains("adresse interne")),
                "{url}: {refused:?}"
            );
        }
        let reached = get_anonymous("http://127.0.0.1:9/x", 1024, false)
            .await
            .unwrap_err();
        assert!(matches!(reached, HttpError::Offline(_)), "{reached:?}");
    }

    #[test]
    fn a_public_only_download_never_goes_through_a_proxy() {
        let proxy = ureq::Proxy::new("http://127.0.0.1:3128").ok();
        assert!(base().proxy(proxy.clone()).build().proxy().is_some());
        assert!(direct(base().proxy(proxy)).proxy().is_none());
        assert!(public_agent().config().proxy().is_none());
    }

    #[test]
    fn internal_addresses_cover_shared_reserved_and_embedded_forms() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "100.64.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "2002:c0a8:101::",
        ] {
            assert!(internal_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["140.82.112.3", "52.216.0.1", "2606:50c0:8000::153"] {
            assert!(!internal_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
