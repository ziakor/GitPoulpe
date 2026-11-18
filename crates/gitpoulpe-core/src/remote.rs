#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_urls: Vec<String>,
    pub push_urls: Vec<String>,
}

impl Remote {
    pub fn push_targets(&self) -> &[String] {
        if self.push_urls.is_empty() {
            &self.fetch_urls
        } else {
            &self.push_urls
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.fetch_urls
            .iter()
            .chain(&self.push_urls)
            .any(|url| url_has_secret(url))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteImpact {
    pub tracking_refs: usize,
    pub upstream_of: Vec<String>,
}

fn authority_range(url: &str) -> Option<(usize, usize)> {
    let start = url.find("://")? + 3;
    let rest = &url[start..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((start, start + end))
}

fn scheme_of(url: &str) -> String {
    url.split_once("://")
        .map_or_else(String::new, |(scheme, _)| scheme.to_ascii_lowercase())
}

fn userinfo_is_secret(url: &str, userinfo: &str) -> bool {
    if userinfo.is_empty() {
        return false;
    }
    scheme_of(url) != "ssh" || userinfo.contains(':')
}

fn trailer_start(url: &str, authority_end: usize) -> Option<usize> {
    url[authority_end..]
        .find(['?', '#'])
        .map(|offset| authority_end + offset)
}

pub fn url_has_secret(url: &str) -> bool {
    let Some((start, end)) = authority_range(url) else {
        return false;
    };
    let userinfo = url[start..end]
        .rsplit_once('@')
        .map(|(userinfo, _)| userinfo);
    userinfo.is_some_and(|userinfo| userinfo_is_secret(url, userinfo))
        || trailer_start(url, end).is_some()
}

pub fn mask_url(url: &str) -> String {
    let Some((start, end)) = authority_range(url) else {
        return url.to_owned();
    };
    let authority = &url[start..end];
    let mut masked = String::from(&url[..start]);
    match authority.rsplit_once('@') {
        Some((userinfo, host)) if userinfo_is_secret(url, userinfo) => {
            masked.push_str("***@");
            masked.push_str(host);
        }
        _ => masked.push_str(authority),
    }
    match trailer_start(url, end) {
        Some(cut) => {
            masked.push_str(&url[end..cut]);
            masked.push_str(&url[cut..=cut]);
            masked.push_str("***");
        }
        None => masked.push_str(&url[end..]),
    }
    masked
}
