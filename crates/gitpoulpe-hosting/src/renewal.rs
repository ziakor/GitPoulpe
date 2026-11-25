use std::time::{SystemTime, UNIX_EPOCH};

use gitpoulpe_core::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::github::GithubError;
use crate::http::{ApiClient, Token};
use crate::secrets::SecretStore;

const MARGIN: u64 = 300;

static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug)]
pub struct Grant {
    pub access: Token,
    pub refresh: Option<Token>,
    pub expires_in: Option<u64>,
    pub refresh_expires_in: Option<u64>,
}

impl Grant {
    pub fn lasting(access: Token) -> Self {
        Self {
            access,
            refresh: None,
            expires_in: None,
            refresh_expires_in: None,
        }
    }

    pub(crate) fn from_reply(value: &Value) -> Option<Self> {
        let token = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(Token::new)
                .filter(|t| !t.is_empty())
        };
        let number = |key: &str| value.get(key).and_then(Value::as_u64);
        Some(Self {
            access: token("access_token")?,
            refresh: token("refresh_token"),
            expires_in: number("expires_in"),
            refresh_expires_in: number("refresh_token_expires_in"),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Renewal {
    refresh_token: String,
    client_id: String,
    access_expires_at: u64,
    refresh_expires_at: Option<u64>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn renewal_key(connection: &Connection) -> String {
    format!("{}#renewal", connection.secret_key())
}

fn read(store: &dyn SecretStore, connection: &Connection) -> Result<Option<Renewal>, GithubError> {
    Ok(store
        .get(&renewal_key(connection))?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

pub(crate) fn save(
    store: &dyn SecretStore,
    connection: &Connection,
    grant: &Grant,
    client_id: &str,
) -> Result<(), GithubError> {
    store.set(&connection.secret_key(), grant.access.expose())?;
    match (&grant.refresh, grant.expires_in) {
        (Some(refresh), Some(expires_in)) => {
            let at = now();
            let renewal = Renewal {
                refresh_token: refresh.expose().to_owned(),
                client_id: client_id.to_owned(),
                access_expires_at: at + expires_in,
                refresh_expires_at: grant.refresh_expires_in.map(|s| at + s),
            };
            let raw = serde_json::to_string(&renewal).map_err(|_| GithubError::Unreadable)?;
            store.set(&renewal_key(connection), &raw)?;
        }
        _ => store.delete(&renewal_key(connection))?,
    }
    Ok(())
}

pub(crate) fn forget(store: &dyn SecretStore, connection: &Connection) -> Result<(), GithubError> {
    store.delete(&renewal_key(connection))?;
    Ok(())
}

pub fn renews_itself(store: &dyn SecretStore, connection: &Connection) -> bool {
    matches!(read(store, connection), Ok(Some(_)))
}

fn stored(store: &dyn SecretStore, connection: &Connection) -> Result<Token, GithubError> {
    store
        .get(&connection.secret_key())?
        .map(|t| Token::new(&t))
        .filter(|t| !t.is_empty())
        .ok_or(GithubError::NotConnected)
}

pub(crate) async fn access_token(
    store: &dyn SecretStore,
    connection: &Connection,
    refused: Option<&Token>,
) -> Result<Token, GithubError> {
    let due = |r: &Renewal| refused.is_some() || now() + MARGIN >= r.access_expires_at;
    match read(store, connection)? {
        Some(renewal) if due(&renewal) => {}
        _ => return stored(store, connection),
    }
    let _turn = RENEWING.lock().await;
    let Some(renewal) = read(store, connection)? else {
        return stored(store, connection);
    };
    let current = stored(store, connection)?;
    let renewed_meanwhile = refused.is_some_and(|t| t.expose() != current.expose());
    if renewed_meanwhile || !due(&renewal) {
        return Ok(current);
    }
    let client = ApiClient::new(&connection.web, None)?.with_plain_json();
    let response = client
        .send(
            "POST",
            "login/oauth/access_token",
            Some(json!({
                "client_id": renewal.client_id,
                "grant_type": "refresh_token",
                "refresh_token": renewal.refresh_token,
            })),
        )
        .await?;
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if let Some(grant) = Grant::from_reply(&value) {
        save(store, connection, &grant, &renewal.client_id)?;
        return Ok(grant.access);
    }
    match value.get("error").and_then(Value::as_str) {
        Some("bad_refresh_token") => {
            let latest = read(store, connection)?;
            if latest.is_some_and(|r| r.refresh_token != renewal.refresh_token) {
                return stored(store, connection);
            }
            forget(store, connection)?;
            Err(GithubError::SessionExpired)
        }
        _ if !response.ok() => Err(GithubError::Server(response.status)),
        _ => Err(GithubError::Unreadable),
    }
}
