#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

fn split_path(path: &str) -> Option<(String, String)> {
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_owned(), repo.to_owned()))
}

fn without_user(authority: &str) -> &str {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
}

fn without_port(host: &str) -> &str {
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

pub fn parse_remote(url: &str) -> Option<RemoteRef> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let authority = without_user(authority);
        let host = match scheme {
            "https" | "http" => authority,
            "ssh" | "git" => without_port(authority),
            _ => return None,
        };
        if host.is_empty() {
            return None;
        }
        let (owner, repo) = split_path(path)?;
        return Some(RemoteRef {
            host: host.to_lowercase(),
            owner,
            repo,
        });
    }
    let (authority, path) = url.split_once(':')?;
    if authority.contains('/') {
        return None;
    }
    let host = without_user(authority);
    if host.len() < 2 {
        return None;
    }
    let (owner, repo) = split_path(path)?;
    Some(RemoteRef {
        host: host.to_lowercase(),
        owner,
        repo,
    })
}
