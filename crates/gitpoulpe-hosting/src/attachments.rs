use std::ops::Range;

use gitpoulpe_core::Connection;

use crate::github::{GithubError, interpret_failure};
use crate::http::{self, HttpError, Response};
use crate::pulls::encode;
use crate::repos::client;
use crate::secrets::SecretStore;

pub const MEDIA_LIMIT: u64 = 20 * 1024 * 1024;
const RAW_CONTENTS: &str = "application/vnd.github.raw+json";
const MAX_REDIRECTS: usize = 3;
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "avif",
];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "ogv", "ogg", "mkv", "avi"];
const SUMMARY_NAME: &str = "<span class=\"m-1\">";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    Video,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Anonymous(String),
    Api(String),
    Contents(String),
}

impl Preview {
    pub fn key(&self) -> &str {
        match self {
            Self::Anonymous(s) | Self::Api(s) | Self::Contents(s) => s,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMedia {
    pub link: String,
    pub before: Option<Preview>,
    pub after: Option<Preview>,
}

pub(crate) struct Change<'a> {
    pub path: &'a str,
    pub previous: Option<&'a str>,
    pub status: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub span: Range<usize>,
    pub kind: AttachmentKind,
    pub name: String,
    pub link: String,
    pub preview: Option<Preview>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Image,
    Video,
    Link,
    Bare,
}

#[derive(Debug)]
struct Reference {
    span: Range<usize>,
    form: Form,
    url: String,
    label: String,
}

fn fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

fn references(body: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut open: Option<(char, usize)> = None;
    let mut next = 0;
    for line in body.split_inclusive('\n') {
        let at = next;
        next += line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        match (open, fence(content)) {
            (None, Some(marker)) => open = Some(marker),
            (Some((c, n)), Some((d, m)))
                if c == d && m >= n && content.trim().chars().all(|x| x == c) =>
            {
                open = None
            }
            (None, None) => scan_line(content, at, &mut found),
            _ => {}
        }
    }
    found
}

fn scan_line(line: &str, at: usize, found: &mut Vec<Reference>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let mut push = |end: usize, form: Form, url: String, label: String| {
            found.push(Reference {
                span: at + i..at + end,
                form,
                url,
                label,
            });
            end
        };
        let next = match bytes[i] {
            b'`' => Some(skip_code(bytes, i)),
            b'!' if bytes.get(i + 1) == Some(&b'[') => {
                markdown(line, i + 1).map(|(end, label, url)| push(end, Form::Image, url, label))
            }
            b'[' if !(bytes.get(i + 1) == Some(&b'!') && bytes.get(i + 2) == Some(&b'[')) => {
                markdown(line, i).map(|(end, label, url)| push(end, Form::Link, url, label))
            }
            b'<' => tag(line, i).map(|(end, form, url, label)| push(end, form, url, label)),
            b'h' if (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
                && (line[i..].starts_with("https://") || line[i..].starts_with("http://")) =>
            {
                let end = bare(line, i);
                Some(push(
                    end,
                    Form::Bare,
                    line[i..end].to_owned(),
                    String::new(),
                ))
            }
            _ => None,
        };
        i = next.unwrap_or(i + 1);
    }
}

fn skip_code(bytes: &[u8], start: usize) -> usize {
    let run = |from: usize| bytes[from..].iter().take_while(|b| **b == b'`').count();
    let n = run(start);
    let mut j = start + n;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let m = run(j);
            if m == n {
                return j + m;
            }
            j += m;
        } else {
            j += 1;
        }
    }
    start + n
}

fn markdown(line: &str, open: usize) -> Option<(usize, String, String)> {
    let close = open + 1 + line[open + 1..].find("](")?;
    let label = &line[open + 1..close];
    if label.contains(']') {
        return None;
    }
    let target = close + 2;
    let rest = &line[target..];
    let (url, after) = if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        (&inner[..end], target + end + 2)
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(rest.len());
        (&rest[..end], target + end)
    };
    let paren = after + line[after..].find(')')?;
    Some((paren + 1, label.trim().to_owned(), url.trim().to_owned()))
}

fn starts_ci(text: &str, word: &str) -> bool {
    text.len() >= word.len() && text.as_bytes()[..word.len()].eq_ignore_ascii_case(word.as_bytes())
}

fn tag(line: &str, open: usize) -> Option<(usize, Form, String, String)> {
    let rest = &line[open + 1..];
    let (form, len) = if starts_ci(rest, "img") {
        (Form::Image, 3)
    } else if starts_ci(rest, "video") {
        (Form::Video, 5)
    } else {
        return None;
    };
    let follow = *rest.as_bytes().get(len)?;
    if !(follow.is_ascii_whitespace() || follow == b'/' || follow == b'>') {
        return None;
    }
    let end = open + 1 + rest.find('>')? + 1;
    let inner = &line[open..end];
    let url = attribute(inner, "src")?;
    Some((end, form, url, attribute(inner, "alt").unwrap_or_default()))
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let at = from + pos;
        from = at + name.len();
        if !lower.as_bytes()[..at]
            .last()
            .is_some_and(u8::is_ascii_whitespace)
        {
            continue;
        }
        let after = lower[from..].trim_start();
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = &tag[tag.len() - value.trim_start().len()..];
        let raw = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or(""),
        };
        return Some(unescape(raw));
    }
    None
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn bare(line: &str, start: usize) -> usize {
    let rest = &line[start..];
    let mut end = start
        + rest
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | ')' | ']')
            })
            .unwrap_or(rest.len());
    while end > start
        && matches!(
            line.as_bytes()[end - 1],
            b'.' | b',' | b';' | b':' | b'!' | b'?'
        )
    {
        end -= 1;
    }
    end
}

fn is_web_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && http::authority(url).is_some()
}

fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.find('/').map_or("", |p| &rest[p..]);
    path.split(['?', '#']).next().unwrap_or("")
}

fn decode_percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&h), Some(&l)) if hex(h).is_some() && hex(l).is_some() => {
                out.push((hex(h).unwrap_or(0) * 16 + hex(l).unwrap_or(0)) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extension_kind(name: &str) -> Option<AttachmentKind> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(AttachmentKind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(AttachmentKind::Video)
    } else {
        None
    }
}

fn uuid_in(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(35)).find_map(|i| {
        let ok = b[i..i + 36].iter().enumerate().all(|(k, c)| {
            if matches!(k, 8 | 13 | 18 | 23) {
                *c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
        ok.then(|| text[i..i + 36].to_ascii_lowercase())
    })
}

fn same_host(url: &str, base: &str) -> bool {
    match (http::authority(url), http::authority(base)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

fn github_content_host(url: &str) -> bool {
    http::authority(url).is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host == "githubusercontent.com" || host.ends_with(".githubusercontent.com")
    })
}

struct Rendered {
    video: bool,
    src: String,
    canonical: Option<String>,
    name: Option<String>,
}

fn rendered_media(html: &str) -> Vec<Rendered> {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find('<') {
        let at = from + pos;
        from = at + 1;
        let video = starts_ci(&lower[at + 1..], "video");
        if !video && !starts_ci(&lower[at + 1..], "img") {
            continue;
        }
        let Some(close) = html[at..].find('>') else {
            break;
        };
        let tag = &html[at..=at + close];
        let Some(src) = attribute(tag, "src") else {
            continue;
        };
        found.push(Rendered {
            video,
            src,
            canonical: attribute(tag, "data-canonical-src"),
            name: if video {
                summary_name(&html[..at])
            } else {
                None
            },
        });
    }
    found
}

fn summary_name(before: &str) -> Option<String> {
    let scope = &before[before.rfind("<details")?..];
    let text = &scope[scope.rfind(SUMMARY_NAME)? + SUMMARY_NAME.len()..];
    Some(unescape(text[..text.find("</span>")?].trim())).filter(|s| !s.is_empty())
}

enum Hosted {
    Asset(String),
    File(String),
    Direct,
}

fn github_target(url: &str, web: &str) -> Option<Hosted> {
    let path = path_of(url);
    if github_content_host(url) {
        return Some(uuid_in(path).map_or(Hosted::Direct, Hosted::Asset));
    }
    if !same_host(url, web) {
        return None;
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["user-attachments", "assets", id] => uuid_in(id).map(Hosted::Asset),
        [_, _, "assets", n, id] if digits(n) => uuid_in(id).map(Hosted::Asset),
        ["user-attachments", "files", n, name] | [_, _, "files", n, name] if digits(n) => {
            Some(Hosted::File(decode_percent(name)))
        }
        _ => None,
    }
}

fn last_segment(url: &str) -> String {
    decode_percent(path_of(url).rsplit('/').next().unwrap_or(""))
}

pub(crate) fn github_attachments(body: &str, html: Option<&str>, web: &str) -> Vec<Attachment> {
    let rendered = html.map(rendered_media).unwrap_or_default();
    references(body)
        .into_iter()
        .filter_map(|r| github_attachment(r, &rendered, web))
        .collect()
}

fn github_attachment(r: Reference, rendered: &[Rendered], web: &str) -> Option<Attachment> {
    if !is_web_url(&r.url) {
        return None;
    }
    let target = github_target(&r.url, web);
    if target.is_none() && !matches!(r.form, Form::Image | Form::Video) {
        return None;
    }
    let shown = match &target {
        Some(Hosted::Asset(uuid)) => rendered
            .iter()
            .find(|m| m.src.to_ascii_lowercase().contains(uuid.as_str())),
        _ => rendered
            .iter()
            .find(|m| m.canonical.as_deref() == Some(r.url.as_str()) || m.src == r.url),
    };
    let kind = if matches!(target, Some(Hosted::File(_))) {
        AttachmentKind::File
    } else if shown.is_some_and(|m| m.video) || r.form == Form::Video {
        AttachmentKind::Video
    } else if shown.is_some()
        || r.form == Form::Image
        || extension_kind(&r.url) == Some(AttachmentKind::Image)
    {
        AttachmentKind::Image
    } else if r.form == Form::Bare && matches!(target, Some(Hosted::Asset(_))) {
        AttachmentKind::Video
    } else {
        AttachmentKind::File
    };
    let label = Some(r.label.clone()).filter(|l| !l.is_empty());
    let name = match (kind, &target) {
        (AttachmentKind::File, Some(Hosted::File(file))) => label.unwrap_or_else(|| file.clone()),
        (AttachmentKind::Video, _) => shown
            .and_then(|m| m.name.clone())
            .or(label)
            .unwrap_or_else(|| "vidéo".to_owned()),
        _ => label.unwrap_or_else(|| last_segment(&r.url)),
    };
    let preview = (kind == AttachmentKind::Image)
        .then(|| match shown {
            Some(m) => Some(m.src.clone()),
            None => target.is_some().then(|| r.url.clone()),
        })
        .flatten()
        .filter(|u| is_web_url(u) && (github_content_host(u) || same_host(u, web)))
        .map(Preview::Anonymous);
    Some(Attachment {
        span: r.span,
        kind,
        name,
        link: r.url,
        preview,
    })
}

struct Upload {
    api: String,
    link: String,
    file: String,
}

fn gitlab_upload(url: &str, web: &str, project: (&str, &str)) -> Option<Upload> {
    let web = web.trim_end_matches('/');
    let path = match url.strip_prefix(web) {
        Some(rest) if rest.starts_with('/') => rest,
        Some(_) => return None,
        None if url.starts_with('/') && !url.starts_with("//") => url,
        None => return None,
    };
    let path = path.split(['?', '#']).next()?;
    let (scope, tail) = path.split_once("/uploads/")?;
    let (secret, file) = tail.split_once('/')?;
    let file = decode_percent(file);
    if !(16..=64).contains(&secret.len())
        || !secret.bytes().all(|b| b.is_ascii_alphanumeric())
        || file.is_empty()
        || file.contains('/')
        || file == "."
        || file == ".."
    {
        return None;
    }
    let own = format!("/{}/{}", project.0, project.1);
    let scope = if scope.is_empty() {
        own.as_str()
    } else {
        scope
    };
    let api_project = match scope.strip_prefix("/-/project/") {
        Some(id) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => id.to_owned(),
        Some(_) => return None,
        None if scope.contains("/-/") => return None,
        None => encode(scope.trim_start_matches('/')),
    };
    let encoded = encode(&file);
    Some(Upload {
        api: format!("projects/{api_project}/uploads/{secret}/{encoded}"),
        link: format!("{web}{scope}/uploads/{secret}/{encoded}"),
        file,
    })
}

pub(crate) fn gitlab_attachments(body: &str, web: &str, project: (&str, &str)) -> Vec<Attachment> {
    references(body)
        .into_iter()
        .filter_map(|r| {
            let label = Some(r.label.clone()).filter(|l| !l.is_empty());
            match gitlab_upload(&r.url, web, project) {
                Some(upload) => {
                    let kind = match extension_kind(&upload.file) {
                        Some(kind) => kind,
                        None if r.form == Form::Video => AttachmentKind::Video,
                        None => AttachmentKind::File,
                    };
                    Some(Attachment {
                        span: r.span,
                        kind,
                        name: label.unwrap_or(upload.file),
                        link: upload.link,
                        preview: (kind == AttachmentKind::Image)
                            .then_some(Preview::Api(upload.api)),
                    })
                }
                None if is_web_url(&r.url) && matches!(r.form, Form::Image | Form::Video) => {
                    Some(Attachment {
                        span: r.span,
                        kind: if r.form == Form::Video {
                            AttachmentKind::Video
                        } else {
                            AttachmentKind::Image
                        },
                        name: label.unwrap_or_else(|| last_segment(&r.url)),
                        link: r.url,
                        preview: None,
                    })
                }
                None => None,
            }
        })
        .collect()
}

fn path_segments(path: &str) -> String {
    path.split('/').map(encode).collect::<Vec<_>>().join("/")
}

pub(crate) fn is_image_path(path: &str) -> bool {
    extension_kind(path) == Some(AttachmentKind::Image)
}

fn file_media(
    change: Change<'_>,
    base: Option<&str>,
    head: &str,
    link: impl Fn(&str, &str) -> String,
    read: impl Fn(&str, &str) -> Preview,
) -> FileMedia {
    let removed = change.status == "removed";
    let image = is_image_path(change.path);
    let before_path = change.previous.unwrap_or(change.path);
    let base = base.filter(|b| !b.is_empty());
    FileMedia {
        link: match (removed, base) {
            (true, Some(b)) => link(b, before_path),
            _ => link(head, change.path),
        },
        before: base
            .filter(|_| image && change.status != "added")
            .map(|b| read(b, before_path)),
        after: (image && !removed).then(|| read(head, change.path)),
    }
}

pub(crate) fn github_file_media(
    web: &str,
    repo: (&str, &str),
    change: Change<'_>,
    base: Option<&str>,
    head: &str,
) -> FileMedia {
    let web = web.trim_end_matches('/');
    let (owner, name) = repo;
    file_media(
        change,
        base,
        head,
        |rev, path| {
            format!(
                "{web}/{owner}/{name}/blob/{}/{}",
                encode(rev),
                path_segments(path)
            )
        },
        |rev, path| {
            Preview::Contents(format!(
                "repos/{owner}/{name}/contents/{}?ref={}",
                path_segments(path),
                encode(rev)
            ))
        },
    )
}

pub(crate) fn gitlab_file_media(
    web: &str,
    repo: (&str, &str),
    change: Change<'_>,
    base: Option<&str>,
    head: &str,
) -> FileMedia {
    let web = web.trim_end_matches('/');
    let (owner, name) = repo;
    let project = encode(&format!("{owner}/{name}"));
    file_media(
        change,
        base,
        head,
        |rev, path| {
            format!(
                "{web}/{owner}/{name}/-/blob/{}/{}",
                encode(rev),
                path_segments(path)
            )
        },
        |rev, path| {
            Preview::Api(format!(
                "projects/{project}/repository/files/{}/raw?ref={}",
                encode(path),
                encode(rev)
            ))
        },
    )
}

pub(crate) fn gitlab_web(api: &str) -> &str {
    api.strip_suffix("/api/v4").unwrap_or(api)
}

fn redirect_target(current: &str, location: &str) -> Option<String> {
    if is_web_url(location) {
        Some(location.to_owned())
    } else if location.starts_with('/') && !location.starts_with("//") {
        Some(format!("{}{location}", http::origin(current)?))
    } else {
        None
    }
}

fn refused(why: &str) -> GithubError {
    GithubError::Http(HttpError::Other(why.to_owned()))
}

pub async fn fetch_preview(
    store: &dyn SecretStore,
    connection: &Connection,
    preview: &Preview,
) -> Result<Vec<u8>, GithubError> {
    let (mut url, mut response): (String, Response) = match preview {
        Preview::Api(path) | Preview::Contents(path) => {
            let accept = if matches!(preview, Preview::Contents(_)) {
                RAW_CONTENTS
            } else {
                "*/*"
            };
            let client = client(store, connection)?;
            let response = client.get_bytes(path, accept, MEDIA_LIMIT).await?;
            (format!("{}/{path}", client.base()), response)
        }
        Preview::Anonymous(url) => {
            if !(github_content_host(url)
                || same_host(url, &connection.web)
                || same_host(url, &connection.api))
            {
                return Err(refused("adresse hors des serveurs du fournisseur refusée"));
            }
            (url.clone(), http::get_anonymous(url, MEDIA_LIMIT).await?)
        }
    };
    let mut hops = 0;
    while (300..400).contains(&response.status) {
        let Some(location) = response.header("location") else {
            break;
        };
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(refused("trop de redirections"));
        }
        url = redirect_target(&url, location).ok_or_else(|| refused("redirection invalide"))?;
        response = http::get_anonymous(&url, MEDIA_LIMIT).await?;
    }
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    Ok(response.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEB: &str = "https://github.com";
    const UUID: &str = "fd0fc972-c7df-42cc-b025-35deb408c0b6";

    fn signed(uuid: &str, ext: &str) -> String {
        format!(
            "https://private-user-images.githubusercontent.com/1/7-{uuid}.{ext}?jwt=a.b&amp;x=1"
        )
    }

    #[test]
    fn references_skip_code_and_keep_byte_spans_in_unicode_text() {
        let body = "é ![shot](https://x.test/a.png \"t\") `![no](u)`\n```\n![no](u)\n```\nfin <IMG alt=\"b\" SRC='https://x.test/b.png'/> https://x.test/c.\n";
        let found = references(body);
        let urls: Vec<&str> = found.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://x.test/a.png",
                "https://x.test/b.png",
                "https://x.test/c"
            ]
        );
        assert_eq!(
            &body[found[0].span.clone()],
            "![shot](https://x.test/a.png \"t\")"
        );
        assert_eq!(
            (found[0].form, found[0].label.as_str()),
            (Form::Image, "shot")
        );
        assert_eq!(
            &body[found[1].span.clone()],
            "<IMG alt=\"b\" SRC='https://x.test/b.png'/>"
        );
        assert_eq!((found[1].form, found[1].label.as_str()), (Form::Image, "b"));
        assert_eq!(found[2].form, Form::Bare);
    }

    #[test]
    fn a_linked_badge_yields_its_image_and_unclosed_syntax_yields_nothing() {
        let found =
            references("[![ci](https://x.test/b.svg)](https://x.test/ci) [broken](nope ![x](");
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].form, found[0].url.as_str()),
            (Form::Image, "https://x.test/b.svg")
        );
        assert_eq!(
            (found[1].form, found[1].url.as_str()),
            (Form::Bare, "https://x.test/ci")
        );
    }

    #[test]
    fn github_images_videos_and_files_map_to_signed_sources_by_uuid() {
        let video = "0a0fbb9c-3fe6-443d-9621-749ebd906115";
        let other = "39be22d3-faa0-4641-b4a5-477dba83845c";
        let body = format!(
            "<img width=\"9\" alt=\"Shot\" src=\"https://github.com/user-attachments/assets/{UUID}\" />\n\nhttps://github.com/user-attachments/assets/{video}\n\n- https://github.com/user-attachments/assets/{other}\n[logs.zip](https://github.com/user-attachments/files/123/logs.zip) [tab](https://github.com/acme/proj/pull/3/files)"
        );
        let html = format!(
            "<a href=\"x\"><img alt=\"Shot\" src=\"{}\"></a><details open><summary><span class=\"m-1\">demo &amp; fix.mp4</span></summary><video src=\"{}\"></video></details>",
            signed(UUID, "png"),
            signed(video, "mp4")
        );
        let found = github_attachments(&body, Some(&html), WEB);
        let kinds: Vec<(AttachmentKind, &str)> =
            found.iter().map(|a| (a.kind, a.name.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (AttachmentKind::Image, "Shot"),
                (AttachmentKind::Video, "demo & fix.mp4"),
                (AttachmentKind::Video, "vidéo"),
                (AttachmentKind::File, "logs.zip"),
            ]
        );
        assert_eq!(
            found[0].preview,
            Some(Preview::Anonymous(
                signed(UUID, "png").replace("&amp;", "&")
            ))
        );
        assert_eq!(
            found[0].link,
            format!("https://github.com/user-attachments/assets/{UUID}")
        );
        assert!(found[1..].iter().all(|a| a.preview.is_none()));
    }

    #[test]
    fn without_rendered_html_a_github_image_falls_back_to_its_stable_address() {
        let body = format!("![x](https://github.com/user-attachments/assets/{UUID})");
        let found = github_attachments(&body, None, WEB);
        assert_eq!(found[0].kind, AttachmentKind::Image);
        assert_eq!(
            found[0].preview,
            Some(Preview::Anonymous(format!(
                "https://github.com/user-attachments/assets/{UUID}"
            )))
        );
    }

    #[test]
    fn external_images_load_only_through_the_github_proxy_and_plain_links_are_not_attachments() {
        let body = "![a](https://evil.test/a.png) ![b](https://evil.test/b.png) [c](https://evil.test/c.png)";
        let html = "<img src=\"https://camo.githubusercontent.com/1/2\" data-canonical-src=\"https://evil.test/a.png\">";
        let found = github_attachments(body, Some(html), WEB);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].preview,
            Some(Preview::Anonymous(
                "https://camo.githubusercontent.com/1/2".to_owned()
            ))
        );
        assert_eq!(
            (found[1].kind, &found[1].preview),
            (AttachmentKind::Image, &None)
        );
        let forged =
            "<img src=\"https://evil.test/x.png\" data-canonical-src=\"https://evil.test/b.png\">";
        assert_eq!(github_attachments(body, Some(forged), WEB)[1].preview, None);
    }

    #[test]
    fn gitlab_uploads_are_read_through_the_project_api_and_others_are_never_fetched() {
        let secret = "0123456789abcdef0123456789abcdef";
        let body = format!(
            "![screen shot](/uploads/{secret}/screen%20shot.png) ![rec](/uploads/{secret}/rec.webm) [notes.pdf](/uploads/{secret}/notes.pdf) ![g](https://gitlab.test/-/project/42/uploads/{secret}/g.gif) ![o](https://gitlab.test/other/grp/proj/uploads/{secret}/o.jpg) ![ext](https://evil.test/e.png) ![bad](/uploads/{secret}/..) [doc](/uploads/short/x.png)"
        );
        let found = gitlab_attachments(&body, "https://gitlab.test", ("grp/sub", "proj"));
        let summary: Vec<(AttachmentKind, &str, Option<&Preview>)> = found
            .iter()
            .map(|a| (a.kind, a.name.as_str(), a.preview.as_ref()))
            .collect();
        let api = |p: &str| Preview::Api(p.to_owned());
        assert_eq!(
            summary,
            [
                (
                    AttachmentKind::Image,
                    "screen shot",
                    Some(&api(&format!(
                        "projects/grp%2Fsub%2Fproj/uploads/{secret}/screen%20shot.png"
                    )))
                ),
                (AttachmentKind::Video, "rec", None),
                (AttachmentKind::File, "notes.pdf", None),
                (
                    AttachmentKind::Image,
                    "g",
                    Some(&api(&format!("projects/42/uploads/{secret}/g.gif")))
                ),
                (
                    AttachmentKind::Image,
                    "o",
                    Some(&api(&format!(
                        "projects/other%2Fgrp%2Fproj/uploads/{secret}/o.jpg"
                    )))
                ),
                (AttachmentKind::Image, "ext", None),
            ]
        );
        assert_eq!(
            found[0].link,
            format!("https://gitlab.test/grp/sub/proj/uploads/{secret}/screen%20shot.png")
        );
    }

    #[test]
    fn redirects_resolve_against_the_current_origin_only() {
        assert_eq!(
            redirect_target("https://a.test/x/y", "/z?q=1").as_deref(),
            Some("https://a.test/z?q=1")
        );
        assert_eq!(redirect_target("https://a.test/x", "//b.test/z"), None);
        assert_eq!(
            redirect_target("https://a.test/x", "file:///etc/passwd"),
            None
        );
    }
}
