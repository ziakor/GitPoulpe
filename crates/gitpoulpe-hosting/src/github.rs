use gitpoulpe_core::{Connection, Provider};

use crate::http::{ApiClient, HttpError, Response, Token, validate_base};
use crate::secrets::{SecretError, SecretStore};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubHost {
    pub api: String,
    pub web: String,
}

impl GithubHost {
    pub fn dotcom() -> Self {
        Self {
            api: "https://api.github.com".to_owned(),
            web: "https://github.com".to_owned(),
        }
    }

    pub fn enterprise(server: &str) -> Result<Self, GithubError> {
        let web = validate_base(server).map_err(GithubError::Http)?;
        let api = format!("{web}/api/v3");
        Ok(Self { api, web })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub login: String,
    pub name: Option<String>,
    pub scopes: Vec<String>,
    pub fine_grained: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GithubError {
    EmptyToken,
    Unauthorized,
    SsoRequired(String),
    RateLimited(Option<String>),
    Forbidden(String),
    NotFound,
    Server(u16),
    Unreadable,
    Http(HttpError),
    Secret(SecretError),
    NotConnected,
    DeviceDenied,
    DeviceExpired,
    ForkPending,
    NameTaken(String),
    Validation(String),
    PullExists(u64, String),
    BranchNotPublished(String),
    AnchorStale { prepared: String, current: String },
    MergeRefused(String),
    MergeNotAllowed(String),
}

impl std::fmt::Display for GithubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyToken => f.write_str("collez un jeton d'accès"),
            Self::Unauthorized => f.write_str(
                "le jeton est expiré, révoqué ou invalide : reconnectez le compte avec un nouveau jeton",
            ),
            Self::SsoRequired(url) => write!(
                f,
                "l'organisation exige une autorisation SSO pour ce jeton : autorisez-le sur {url}, puis réessayez"
            ),
            Self::RateLimited(Some(reset)) => write!(
                f,
                "limite de requêtes atteinte ; réessayez après {reset} (UTC, secondes Unix)"
            ),
            Self::RateLimited(None) => f.write_str("limite de requêtes atteinte ; réessayez plus tard"),
            Self::Forbidden(why) => write!(f, "accès refusé : {why}"),
            Self::NotFound => f.write_str("ressource introuvable ou non autorisée pour ce jeton"),
            Self::Server(status) => write!(f, "le serveur a répondu {status}"),
            Self::Unreadable => f.write_str("réponse du serveur illisible"),
            Self::Http(e) => write!(f, "{e}"),
            Self::Secret(e) => write!(f, "{e}"),
            Self::NotConnected => f.write_str("aucun jeton n'est enregistré pour cette connexion"),
            Self::ForkPending => f.write_str(
                "le fork est en cours de création chez GitHub : réessayez dans un instant",
            ),
            Self::NameTaken(name) => write!(
                f,
                "{name} existe déjà et n'est pas un fork de ce dépôt : choisissez une autre destination"
            ),
            Self::Validation(why) => write!(f, "GitHub refuse la demande : {why}"),
            Self::PullExists(number, url) => write!(
                f,
                "une pull request est déjà ouverte pour cette branche : #{number} ({url})"
            ),
            Self::BranchNotPublished(branch) => write!(
                f,
                "la branche {branch} n'est pas publiée sur le dépôt distant : publiez-la d'abord"
            ),
            Self::AnchorStale { prepared, current } => write!(
                f,
                "la pull request a été mise à jour ({prepared} → {current}) : l'ancrage du commentaire est périmé, relisez le diff avant de publier"
            ),
            Self::MergeRefused(why) => write!(f, "fusion refusée par GitHub : {why}"),
            Self::MergeNotAllowed(why) => write!(f, "fusion impossible : {why}"),
            Self::DeviceDenied => f.write_str("l'autorisation a été refusée dans le navigateur"),
            Self::DeviceExpired => {
                f.write_str("le code a expiré avant l'autorisation : recommencez la connexion")
            }
        }
    }
}

impl std::error::Error for GithubError {}

impl From<HttpError> for GithubError {
    fn from(error: HttpError) -> Self {
        Self::Http(error)
    }
}

impl From<SecretError> for GithubError {
    fn from(error: SecretError) -> Self {
        Self::Secret(error)
    }
}

pub(crate) fn interpret_failure(response: &Response) -> GithubError {
    let message = response
        .json()
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str().map(str::to_owned)))
        .unwrap_or_default();
    match response.status {
        401 => GithubError::Unauthorized,
        403 | 429 => {
            if let Some(sso) = response.header("x-github-sso")
                && let Some(url) = sso.split("url=").nth(1)
            {
                return GithubError::SsoRequired(url.trim().to_owned());
            }
            if response.header("x-ratelimit-remaining") == Some("0") || response.status == 429 {
                return GithubError::RateLimited(
                    response.header("x-ratelimit-reset").map(str::to_owned),
                );
            }
            GithubError::Forbidden(if message.is_empty() {
                "droits insuffisants".to_owned()
            } else {
                message
            })
        }
        404 => GithubError::NotFound,
        405 | 409 => GithubError::MergeRefused(if message.is_empty() {
            format!("statut {}", response.status)
        } else {
            message
        }),
        422 => GithubError::Validation(validation_message(response, &message)),
        status => GithubError::Server(status),
    }
}

fn validation_message(response: &Response, fallback: &str) -> String {
    let detail = response
        .json()
        .ok()
        .and_then(|v| v.get("errors")?.as_array().cloned())
        .and_then(|errors| {
            errors.iter().find_map(|e| {
                e.get("message")
                    .and_then(|m| m.as_str())
                    .or_else(|| e.as_str())
                    .map(str::to_owned)
            })
        });
    match (detail, fallback.is_empty()) {
        (Some(d), false) => format!("{fallback} ({d})"),
        (Some(d), true) => d,
        (None, false) => fallback.to_owned(),
        (None, true) => "données invalides".to_owned(),
    }
}

fn parse_session(response: &Response) -> Result<Session, GithubError> {
    let user = response.json().map_err(|_| GithubError::Unreadable)?;
    let login = user
        .get("login")
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .ok_or(GithubError::Unreadable)?
        .to_owned();
    let scopes_header = response.header("x-oauth-scopes");
    Ok(Session {
        login,
        name: user.get("name").and_then(|v| v.as_str()).map(str::to_owned),
        scopes: scopes_header
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        fine_grained: scopes_header.is_none(),
    })
}

async fn current_user(api: &str, token: &Token) -> Result<Session, GithubError> {
    let client = ApiClient::new(api, Some(token))?;
    let response = client.get("user").await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    parse_session(&response)
}

fn connection_id(host: &GithubHost, login: &str) -> String {
    let host = host
        .web
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    format!("{host}/{}", login.to_lowercase())
}

pub async fn connect_github(
    store: &dyn SecretStore,
    host: &GithubHost,
    token: &str,
) -> Result<(Connection, Session), GithubError> {
    let token = Token::new(token);
    if token.is_empty() {
        return Err(GithubError::EmptyToken);
    }
    let session = current_user(&host.api, &token).await?;
    let connection = Connection {
        id: connection_id(host, &session.login),
        provider: Provider::Github,
        api: host.api.clone(),
        web: host.web.clone(),
        login: session.login.clone(),
        scopes: session.scopes.clone(),
    };
    store.set(&connection.secret_key(), token.expose())?;
    Ok((connection, session))
}

pub async fn check_connection(
    store: &dyn SecretStore,
    connection: &Connection,
) -> Result<Session, GithubError> {
    let token = store
        .get(&connection.secret_key())?
        .map(|t| Token::new(&t))
        .ok_or(GithubError::NotConnected)?;
    let session = current_user(&connection.api, &token).await?;
    if session.login != connection.login {
        return Err(GithubError::Forbidden(
            "le jeton appartient à un autre compte que cette connexion".to_owned(),
        ));
    }
    Ok(session)
}

pub fn disconnect(store: &dyn SecretStore, connection: &Connection) -> Result<(), GithubError> {
    store.delete(&connection.secret_key())?;
    Ok(())
}
