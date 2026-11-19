use gitpoulpe_core::{Connection, Provider};
use serde_json::Value;

use crate::github::{GithubError, Session, interpret_failure};
use crate::http::{ApiClient, Token, validate_base};
use crate::secrets::SecretStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitlabHost {
    pub api: String,
    pub web: String,
}

impl GitlabHost {
    pub fn dotcom() -> Self {
        Self {
            api: "https://gitlab.com/api/v4".to_owned(),
            web: "https://gitlab.com".to_owned(),
        }
    }

    pub fn self_managed(server: &str) -> Result<Self, GithubError> {
        let web = validate_base(server).map_err(GithubError::Http)?;
        let api = format!("{web}/api/v4");
        Ok(Self { api, web })
    }
}

async fn current_user(api: &str, token: &Token) -> Result<Session, GithubError> {
    let client = ApiClient::new(api, Some(token))?;
    let response = client.get("user").await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let user = response.json().map_err(|_| GithubError::Unreadable)?;
    let login = user
        .get("username")
        .and_then(Value::as_str)
        .filter(|l| !l.is_empty())
        .ok_or(GithubError::Unreadable)?
        .to_owned();
    let name = user.get("name").and_then(Value::as_str).map(str::to_owned);
    let own = client.get("personal_access_tokens/self").await?;
    let (scopes, expires_at) = match own.status {
        404 => (Vec::new(), None),
        _ if own.ok() => {
            let value = own.json().map_err(|_| GithubError::Unreadable)?;
            let scopes = value
                .get("scopes")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let expires = value
                .get("expires_at")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (scopes, expires)
        }
        _ => return Err(interpret_failure(&own)),
    };
    Ok(Session {
        login,
        name,
        scopes,
        fine_grained: false,
        expires_at,
    })
}

fn connection_id(host: &GitlabHost, login: &str) -> String {
    let host = host
        .web
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    format!("{host}/{}", login.to_lowercase())
}

pub async fn connect_gitlab(
    store: &dyn SecretStore,
    host: &GitlabHost,
    token: &str,
) -> Result<(Connection, Session), GithubError> {
    let token = Token::new(token);
    if token.is_empty() {
        return Err(GithubError::EmptyToken);
    }
    let session = current_user(&host.api, &token).await?;
    let connection = Connection {
        id: connection_id(host, &session.login),
        provider: Provider::Gitlab,
        api: host.api.clone(),
        web: host.web.clone(),
        login: session.login.clone(),
        scopes: session.scopes.clone(),
    };
    store.set(&connection.secret_key(), token.expose())?;
    Ok((connection, session))
}

pub async fn check_gitlab(
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
