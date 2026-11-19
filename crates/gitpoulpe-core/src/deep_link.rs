pub const LINK_SCHEME: &str = "gitpoulpe://open/";
const MAX_LINK: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Repository,
    Commit(String),
    Pull(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeepLink {
    pub host: String,
    pub path: String,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    TooLong,
    Scheme,
    Forbidden,
    Host,
    Path,
    Target,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "le lien est vide",
            Self::TooLong => "le lien est trop long",
            Self::Scheme => "lien non reconnu : attendu gitpoulpe://open/<hôte>/<dépôt>… ou une adresse https du fournisseur",
            Self::Forbidden => "le lien contient des caractères, des paramètres ou une action non acceptés",
            Self::Host => "hôte invalide dans le lien",
            Self::Path => "chemin de dépôt invalide dans le lien",
            Self::Target => "objet visé invalide : attendu un commit ou une demande de fusion",
        })
    }
}

impl std::error::Error for LinkError {}

fn valid_host(host: &str) -> bool {
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && port
            .is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.len() <= 100
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn strip_git(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

pub fn parse_link(input: &str) -> Result<DeepLink, LinkError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(LinkError::Empty);
    }
    if input.len() > MAX_LINK {
        return Err(LinkError::TooLong);
    }
    let rest = input
        .strip_prefix(LINK_SCHEME)
        .or_else(|| input.strip_prefix("https://"))
        .ok_or(LinkError::Scheme)?;
    if rest
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '%' | '\\' | '@'))
    {
        return Err(LinkError::Forbidden);
    }
    let rest = rest.trim_end_matches('/');
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or_default().to_ascii_lowercase();
    if !valid_host(&host) {
        return Err(LinkError::Host);
    }
    let segments: Vec<&str> = parts.collect();
    let marker = segments
        .iter()
        .position(|s| matches!(*s, "commit" | "commits" | "pull" | "merge_requests" | "-"));
    let (repo, tail) = match marker {
        Some(index) => (&segments[..index], &segments[index..]),
        None => (&segments[..], &segments[0..0]),
    };
    if repo.len() < 2 || !repo.iter().all(|s| valid_segment(s)) {
        return Err(LinkError::Path);
    }
    let mut path: Vec<String> = repo.iter().map(|s| (*s).to_owned()).collect();
    if let Some(last) = path.last_mut() {
        *last = strip_git(last).to_owned();
        if last.is_empty() {
            return Err(LinkError::Path);
        }
    }
    let tail: &[&str] = if tail.first() == Some(&"-") {
        &tail[1..]
    } else {
        tail
    };
    let target = match tail {
        [] => LinkTarget::Repository,
        ["commit" | "commits", sha]
            if (7..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            LinkTarget::Commit(sha.to_ascii_lowercase())
        }
        ["pull" | "merge_requests", number]
            if !number.is_empty()
                && number.len() <= 9
                && number.bytes().all(|b| b.is_ascii_digit())
                && number.parse::<u64>().is_ok_and(|n| n > 0) =>
        {
            LinkTarget::Pull(number.parse().unwrap_or(0))
        }
        _ => return Err(LinkError::Target),
    };
    Ok(DeepLink {
        host,
        path: path.join("/"),
        target,
    })
}

pub fn remote_host_path(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim();
    let after = if let Some((_, rest)) = remote.split_once("://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host, path) = rest.split_once('/')?;
        let host = host
            .split_once(':')
            .filter(|(_, port)| {
                remote.starts_with("ssh://") && port.bytes().all(|b| b.is_ascii_digit())
            })
            .map_or(host, |(name, _)| name);
        (host.to_owned(), path.to_owned())
    } else {
        let rest = remote.rsplit_once('@').map_or(remote, |(_, r)| r);
        let (host, path) = rest.split_once(':')?;
        (host.to_owned(), path.to_owned())
    };
    let path = after.1.trim_matches('/');
    Some((
        after.0.to_ascii_lowercase(),
        strip_git(path).to_ascii_lowercase(),
    ))
}

impl DeepLink {
    pub fn label(&self) -> String {
        match &self.target {
            LinkTarget::Repository => format!("{}/{}", self.host, self.path),
            LinkTarget::Commit(sha) => {
                format!(
                    "{}/{} · commit {}",
                    self.host,
                    self.path,
                    &sha[..sha.len().min(10)]
                )
            }
            LinkTarget::Pull(n) => format!("{}/{} · demande #{n}", self.host, self.path),
        }
    }

    pub fn matches_remote(&self, remote: &str) -> bool {
        let link_host = self
            .host
            .split_once(':')
            .map_or(self.host.as_str(), |(h, _)| h);
        remote_host_path(remote)
            .is_some_and(|(host, path)| host == link_host && path == self.path.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> DeepLink {
        parse_link(input).unwrap_or_else(|e| panic!("{input}: {e}"))
    }

    #[test]
    fn valid_links_resolve_to_host_path_and_target() {
        let repo = ok("gitpoulpe://open/github.com/ziakor/gitpoulpe");
        assert_eq!(
            (repo.host.as_str(), repo.path.as_str(), &repo.target),
            ("github.com", "ziakor/gitpoulpe", &LinkTarget::Repository)
        );
        assert_eq!(
            ok("https://GitHub.com/ziakor/gitpoulpe/pull/42/").target,
            LinkTarget::Pull(42)
        );
        assert_eq!(
            ok("https://github.com/o/r/commit/ABCDEF1234").target,
            LinkTarget::Commit("abcdef1234".into())
        );
        let gitlab = ok("https://gitlab.example.com:8443/group/sub/project.git/-/merge_requests/7");
        assert_eq!(gitlab.host, "gitlab.example.com:8443");
        assert_eq!(gitlab.path, "group/sub/project");
        assert_eq!(gitlab.target, LinkTarget::Pull(7));
        assert_eq!(
            ok("https://gitlab.com/g/p/-/commit/0123456").target,
            LinkTarget::Commit("0123456".into())
        );
    }

    #[test]
    fn malformed_and_hostile_links_are_refused() {
        let cases: &[(&str, LinkError)] = &[
            ("", LinkError::Empty),
            ("ftp://github.com/o/r", LinkError::Scheme),
            ("javascript:alert(1)", LinkError::Scheme),
            ("file:///etc/passwd", LinkError::Scheme),
            ("https://github.com/o/r?action=delete", LinkError::Forbidden),
            ("https://github.com/o/r#x", LinkError::Forbidden),
            ("https://user@github.com/o/r", LinkError::Forbidden),
            ("https://github.com/o/r%2F..", LinkError::Forbidden),
            ("https://github.com/o/r\nx", LinkError::Forbidden),
            ("https://git hub.com/o/r", LinkError::Forbidden),
            ("https://github.com/o/$(id)", LinkError::Path),
            ("https://github.com/o/r;rm", LinkError::Path),
            ("https://-bad.com/o/r", LinkError::Host),
            ("https://github.com:abc/o/r", LinkError::Host),
            ("https:///o/r", LinkError::Host),
            ("https://github.com/o", LinkError::Path),
            ("https://github.com/../r", LinkError::Path),
            ("https://github.com/o/r/pull/0", LinkError::Target),
            ("https://github.com/o/r/pull/x", LinkError::Target),
            ("https://github.com/o/r/pull/1/files", LinkError::Target),
            ("https://github.com/o/r/commit/xyz", LinkError::Target),
            ("https://github.com/o/r/commit/abc", LinkError::Target),
            ("https://github.com/o/r/pull/99999999999", LinkError::Target),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_link(input), Err(expected.clone()), "{input}");
        }
        let long = format!("https://github.com/o/{}", "r".repeat(600));
        assert_eq!(parse_link(&long), Err(LinkError::TooLong));
    }

    #[test]
    fn remotes_match_by_host_and_path_across_url_styles() {
        let link = ok("https://github.com/Ziakor/GitPoulpe/pull/3");
        for remote in [
            "https://github.com/ziakor/gitpoulpe.git",
            "git@github.com:ziakor/gitpoulpe.git",
            "ssh://git@github.com/ziakor/gitpoulpe",
            "ssh://git@github.com:22/ziakor/gitpoulpe.git",
        ] {
            assert!(link.matches_remote(remote), "{remote}");
        }
        for remote in [
            "https://gitlab.com/ziakor/gitpoulpe.git",
            "https://github.com/ziakor/other.git",
            "https://github.com/other/gitpoulpe.git",
            "/local/path/gitpoulpe",
            "",
        ] {
            assert!(!link.matches_remote(remote), "{remote}");
        }
        assert_eq!(
            remote_host_path("git@gitlab.com:g/s/p.git"),
            Some(("gitlab.com".into(), "g/s/p".into()))
        );
    }
}
