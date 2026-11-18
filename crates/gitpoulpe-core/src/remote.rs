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

pub fn url_has_secret(url: &str) -> bool {
    let Some((start, end)) = authority_range(url) else {
        return false;
    };
    let authority = &url[start..end];
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

pub fn mask_url(url: &str) -> String {
    let Some((start, end)) = authority_range(url) else {
        return url.to_owned();
    };
    let authority = &url[start..end];
    match authority.rsplit_once('@') {
        Some((userinfo, host)) if userinfo.contains(':') => {
            let user = userinfo.split(':').next().unwrap_or_default();
            format!("{}{user}:***@{host}{}", &url[..start], &url[end..])
        }
        _ => url.to_owned(),
    }
}
