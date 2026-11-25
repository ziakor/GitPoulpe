use std::future::Future;
use std::time::Duration;

use serde_json::{Value, json};

use crate::github::{GithubError, GithubHost};
use crate::http::{ApiClient, validate_base};
use crate::renewal::Grant;

pub const GITHUB_CLIENT_ID: &str = "Ov23litIXPvCD0wH5guL";
const SCOPES: &str = "repo read:org";
const MIN_INTERVAL: u64 = 5;
const MAX_WAIT: u64 = 15 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug)]
pub enum DevicePoll {
    Pending,
    SlowDown(u64),
    Authorized(Grant),
    Denied,
    Expired,
}

fn client(host: &GithubHost) -> Result<ApiClient, GithubError> {
    Ok(ApiClient::new(&host.web, None)?.with_plain_json())
}

fn failure(value: &Value) -> GithubError {
    let code = value.get("error").and_then(Value::as_str).unwrap_or("");
    match code {
        "device_flow_disabled" => GithubError::Forbidden(
            "le flux d'appareil n'est pas activé pour cette application OAuth".to_owned(),
        ),
        "incorrect_client_credentials" => GithubError::Forbidden(
            "identifiant d'application OAuth inconnu de ce serveur".to_owned(),
        ),
        _ => GithubError::Unreadable,
    }
}

pub async fn start_device_flow(
    host: &GithubHost,
    client_id: &str,
) -> Result<DeviceCode, GithubError> {
    let response = client(host)?
        .send(
            "POST",
            "login/device/code",
            Some(json!({"client_id": client_id, "scope": SCOPES})),
        )
        .await?;
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if !response.ok() || value.get("error").is_some() {
        return Err(failure(&value));
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let number = |key: &str| value.get(key).and_then(Value::as_u64);
    let verification_uri = text("verification_uri").ok_or(GithubError::Unreadable)?;
    validate_base(&verification_uri).map_err(|_| GithubError::Unreadable)?;
    Ok(DeviceCode {
        device_code: text("device_code").ok_or(GithubError::Unreadable)?,
        user_code: text("user_code").ok_or(GithubError::Unreadable)?,
        verification_uri,
        expires_in: number("expires_in").unwrap_or(MAX_WAIT).min(MAX_WAIT),
        interval: number("interval").unwrap_or(MIN_INTERVAL).max(1),
    })
}

pub async fn poll_device_flow(
    host: &GithubHost,
    client_id: &str,
    device_code: &str,
) -> Result<DevicePoll, GithubError> {
    let response = client(host)?
        .send(
            "POST",
            "login/oauth/access_token",
            Some(json!({
                "client_id": client_id,
                "device_code": device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            })),
        )
        .await?;
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if value.get("access_token").is_some() {
        return Grant::from_reply(&value)
            .map(DevicePoll::Authorized)
            .ok_or(GithubError::Unreadable);
    }
    match value.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Ok(DevicePoll::Pending),
        Some("slow_down") => Ok(DevicePoll::SlowDown(
            value
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(MIN_INTERVAL + 5),
        )),
        Some("expired_token") => Ok(DevicePoll::Expired),
        Some("access_denied") => Ok(DevicePoll::Denied),
        _ => Err(failure(&value)),
    }
}

pub async fn await_device_token<S, F>(
    host: &GithubHost,
    client_id: &str,
    code: &DeviceCode,
    sleep: S,
) -> Result<Grant, GithubError>
where
    S: Fn(Duration) -> F,
    F: Future<Output = ()>,
{
    let mut interval = code.interval;
    let mut waited = 0;
    loop {
        if waited >= code.expires_in {
            return Err(GithubError::DeviceExpired);
        }
        sleep(Duration::from_secs(interval)).await;
        waited += interval;
        match poll_device_flow(host, client_id, &code.device_code).await? {
            DevicePoll::Authorized(grant) => return Ok(grant),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown(next) => interval = next.max(interval),
            DevicePoll::Denied => return Err(GithubError::DeviceDenied),
            DevicePoll::Expired => return Err(GithubError::DeviceExpired),
        }
    }
}
