mod device;
mod forks;
mod github;
mod http;
mod pulls;
mod remote_url;
mod repos;
mod secrets;

pub use device::{
    DeviceCode, DevicePoll, GITHUB_CLIENT_ID, await_device_token, poll_device_flow,
    start_device_flow,
};
pub use forks::{ForkOutcome, create_fork};
pub use github::{GithubError, GithubHost, Session, check_connection, connect_github, disconnect};
pub use http::{ApiClient, HttpError, Response, Token, next_link, validate_base};
pub use pulls::{
    ChangedFile, Comparison, HostedPull, PullDraft, PullTarget, create_pull_request, prepare_pull,
};
pub use remote_url::{RemoteRef, parse_remote};
pub use repos::{Listing, Organization, RepoSummary, list_organizations, list_repositories};
pub use secrets::{KeyringStore, MemoryStore, SecretError, SecretStore};
