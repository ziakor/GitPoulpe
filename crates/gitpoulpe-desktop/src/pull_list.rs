use gitpoulpe_hosting::{
    LineAnchor, PullComments, PullDetail, PullFilter, PullSummary, Side, list_comments, list_pulls,
    load_pull, post_comment, post_line_comment, reply_to_comment,
};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinePick {
    pub path: String,
    pub line: u64,
    pub start: Option<u64>,
    pub side: Side,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    General,
    Line,
    Reply,
}

pub struct PullListState {
    pub comments: PullComments,
    pub comments_request: u64,
    pub comment_request: u64,
    pub pick: Option<LinePick>,
    pub range_mode: bool,
    pub range_start: Option<LinePick>,
    pub compose: TextEdit,
    pub reply_to: Option<u64>,
    pub sending: bool,
    pub send_error: Option<String>,
    pub send_notice: Option<String>,
    pub unverified: Option<String>,
    pub verifying: bool,
    pub open: bool,
    pub filter: PullFilter,
    pub items: Vec<PullSummary>,
    pub truncated: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<u64>,
    pub detail: Option<PullDetail>,
    pub detail_loading: bool,
    pub detail_error: Option<String>,
    pub notice: Option<String>,
    pub list_request: u64,
    pub detail_request: u64,
    pub version: u32,
}

impl Default for PullListState {
    fn default() -> Self {
        Self {
            comments: PullComments::default(),
            comments_request: 0,
            comment_request: 0,
            pick: None,
            range_mode: false,
            range_start: None,
            compose: TextEdit::new(""),
            reply_to: None,
            sending: false,
            send_error: None,
            send_notice: None,
            unverified: None,
            verifying: false,
            open: false,
            filter: PullFilter::Open,
            items: Vec::new(),
            truncated: false,
            loading: false,
            error: None,
            selected: None,
            detail: None,
            detail_loading: false,
            detail_error: None,
            notice: None,
            list_request: 0,
            detail_request: 0,
            version: 0,
        }
    }
}

impl State {
    fn pull_list_target(&self) -> Option<(gitpoulpe_core::Connection, String, String)> {
        let candidate = self.pulls.candidates.get(self.pulls.choice)?;
        let (owner, name) = match &self.pulls.target {
            Some(target) => (
                target.target_repo.owner.clone(),
                target.target_repo.name.clone(),
            ),
            None => (
                candidate.reference.owner.clone(),
                candidate.reference.repo.clone(),
            ),
        };
        Some((candidate.connection.clone(), owner, name))
    }

    pub fn open_pull_list(&mut self) {
        if self.pull_list_target().is_none() {
            return;
        }
        let list_request = self.pulls.list.list_request + 1;
        let detail_request = self.pulls.list.detail_request + 1;
        let comments_request = self.pulls.list.comments_request + 1;
        let comment_request = self.pulls.list.comment_request + 1;
        self.pulls.list = PullListState {
            open: true,
            list_request,
            detail_request,
            comments_request,
            comment_request,
            ..PullListState::default()
        };
        self.reload_pull_list();
    }

    pub fn close_pull_list(&mut self) {
        let list = &mut self.pulls.list;
        list.open = false;
        list.list_request += 1;
        list.detail_request += 1;
        list.comments_request += 1;
        list.comment_request += 1;
        list.version += 1;
    }

    pub fn set_pull_filter(&mut self, filter: PullFilter) {
        if self.pulls.list.filter == filter {
            return;
        }
        self.pulls.list.filter = filter;
        self.reload_pull_list();
    }

    pub fn reload_pull_list(&mut self) {
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let list = &mut self.pulls.list;
        list.list_request += 1;
        list.loading = true;
        list.error = None;
        list.version += 1;
        let request = list.list_request;
        let filter = list.filter;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_pulls(secrets.as_ref(), &connection, &owner, &name, filter)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullListed { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_listed(
        &mut self,
        request: u64,
        result: Result<(Vec<PullSummary>, bool), String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || request != list.list_request {
            return false;
        }
        list.loading = false;
        match result {
            Ok((items, truncated)) => {
                list.items = items;
                list.truncated = truncated;
            }
            Err(message) => {
                list.items.clear();
                list.error = Some(message);
            }
        }
        list.version += 1;
        true
    }

    pub fn select_pull(&mut self, number: u64) {
        let previous = self.pulls.list.selected;
        self.pulls.list.selected = Some(number);
        if previous != Some(number) {
            let list = &mut self.pulls.list;
            list.detail = None;
            list.notice = None;
            list.comments = PullComments::default();
            list.comments_request += 1;
            list.comment_request += 1;
            list.pick = None;
            list.range_start = None;
            list.reply_to = None;
            list.sending = false;
            list.send_error = None;
            list.send_notice = None;
            list.unverified = None;
            list.verifying = false;
            list.compose = TextEdit::new("");
        }
        self.load_pull_detail();
    }

    pub fn load_pull_detail(&mut self) {
        let Some(number) = self.pulls.list.selected else {
            return;
        };
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let list = &mut self.pulls.list;
        list.detail_request += 1;
        list.detail_loading = true;
        list.detail_error = None;
        list.version += 1;
        let request = list.detail_request;
        self.reload_pull_comments();
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = load_pull(secrets.as_ref(), &connection, &owner, &name, number)
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullLoaded {
                request,
                number,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_loaded(
        &mut self,
        request: u64,
        number: u64,
        result: Result<Box<PullDetail>, String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || request != list.detail_request || list.selected != Some(number) {
            return false;
        }
        list.detail_loading = false;
        match result {
            Ok(detail) => {
                let moved = list
                    .detail
                    .as_ref()
                    .filter(|old| old.summary.number == number)
                    .filter(|old| old.summary.head_sha != detail.summary.head_sha)
                    .map(|old| {
                        format!(
                            "La pull request a été mise à jour depuis votre lecture ({} → {}).",
                            short(&old.summary.head_sha),
                            short(&detail.summary.head_sha)
                        )
                    });
                list.notice = moved.or_else(|| {
                    detail.changed_during_load.then(|| {
                        "La pull request a changé pendant le chargement : actualisez pour relire une version cohérente.".to_owned()
                    })
                });
                list.detail = Some(*detail);
            }
            Err(message) => {
                list.detail = None;
                list.detail_error = Some(message);
            }
        }
        list.version += 1;
        true
    }
}

impl State {
    pub fn reload_pull_comments(&mut self) {
        let Some(number) = self.pulls.list.selected else {
            return;
        };
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        self.pulls.list.comments_request += 1;
        let request = self.pulls.list.comments_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_comments(secrets.as_ref(), &connection, &owner, &name, number)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullCommentsLoaded {
                request,
                number,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_comments(
        &mut self,
        request: u64,
        number: u64,
        result: Result<PullComments, String>,
    ) -> bool {
        let login = self
            .pulls
            .candidates
            .get(self.pulls.choice)
            .map(|c| c.connection.login.clone())
            .unwrap_or_default();
        let list = &mut self.pulls.list;
        if !list.open || request != list.comments_request || list.selected != Some(number) {
            return false;
        }
        match result {
            Ok(comments) => {
                if list.verifying {
                    list.verifying = false;
                    let text = list.compose.text.clone();
                    if comments.contains(&login, &text) {
                        list.send_notice =
                            Some("Le commentaire est déjà publié : ne le renvoyez pas.".to_owned());
                        list.send_error = None;
                        list.unverified = None;
                        list.compose = TextEdit::new("");
                    } else {
                        list.send_notice = Some(
                            "Le commentaire n'a pas été publié : vous pouvez réessayer.".to_owned(),
                        );
                        list.unverified = None;
                    }
                }
                list.comments = comments;
            }
            Err(message) => {
                list.verifying = false;
                list.send_error = Some(message);
            }
        }
        list.version += 1;
        true
    }

    pub fn pull_pick_line(&mut self, path: String, line: u64, side: Side) {
        let list = &mut self.pulls.list;
        list.reply_to = None;
        if list.range_mode {
            match list.range_start.take() {
                Some(start) if start.path == path && start.side == side && line >= start.line => {
                    list.pick = Some(LinePick {
                        path,
                        line,
                        start: Some(start.line),
                        side,
                    });
                }
                _ => {
                    list.range_start = Some(LinePick {
                        path: path.clone(),
                        line,
                        start: None,
                        side,
                    });
                    list.pick = Some(LinePick {
                        path,
                        line,
                        start: None,
                        side,
                    });
                }
            }
        } else {
            list.pick = Some(LinePick {
                path,
                line,
                start: None,
                side,
            });
        }
        list.version += 1;
    }

    pub fn pull_toggle_range(&mut self) {
        let list = &mut self.pulls.list;
        list.range_mode = !list.range_mode;
        list.range_start = None;
        list.version += 1;
    }

    pub fn pull_reply_to(&mut self, id: u64) {
        let list = &mut self.pulls.list;
        list.reply_to = Some(id);
        list.pick = None;
        list.version += 1;
    }

    pub fn pull_send_comment(&mut self, kind: CommentKind) {
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let list = &mut self.pulls.list;
        let Some(number) = list.selected else {
            return;
        };
        if list.sending {
            return;
        }
        let body = list.compose.text.clone();
        let head = list.detail.as_ref().map(|d| d.summary.head_sha.clone());
        let pick = list.pick.clone();
        let reply_to = list.reply_to;
        list.comment_request += 1;
        list.sending = true;
        list.send_error = None;
        list.send_notice = None;
        list.unverified = None;
        list.version += 1;
        let request = list.comment_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let store = secrets.as_ref();
            let result = match kind {
                CommentKind::General => {
                    post_comment(store, &connection, &owner, &name, number, &body)
                        .await
                        .map(|_| ())
                }
                CommentKind::Reply => match reply_to {
                    Some(id) => {
                        reply_to_comment(store, &connection, &owner, &name, number, id, &body)
                            .await
                            .map(|_| ())
                    }
                    None => Err(gitpoulpe_hosting::GithubError::NotFound),
                },
                CommentKind::Line => match (pick, head) {
                    (Some(pick), Some(head)) => {
                        let anchor = LineAnchor {
                            commit_sha: head,
                            path: pick.path,
                            line: pick.line,
                            start_line: pick.start,
                            side: pick.side,
                        };
                        post_line_comment(store, &connection, &owner, &name, number, &anchor, &body)
                            .await
                            .map(|_| ())
                    }
                    _ => Err(gitpoulpe_hosting::GithubError::NotFound),
                },
            };
            let unknown = matches!(
                result,
                Err(gitpoulpe_hosting::GithubError::Http(_)
                    | gitpoulpe_hosting::GithubError::Server(_))
            );
            let _ = sender.send(Event::PullCommentSent {
                request,
                number,
                unknown,
                result: result.map_err(|e| e.to_string()),
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_comment_sent(
        &mut self,
        request: u64,
        number: u64,
        unknown: bool,
        result: Result<(), String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || request != list.comment_request || list.selected != Some(number) {
            return false;
        }
        list.sending = false;
        match result {
            Ok(()) => {
                list.compose = TextEdit::new("");
                list.pick = None;
                list.range_start = None;
                list.reply_to = None;
                list.send_notice = Some("Commentaire publié.".to_owned());
                self.reload_pull_comments();
            }
            Err(message) => {
                list.send_error = Some(message);
                list.unverified = unknown.then(|| list.compose.text.clone());
            }
        }
        self.pulls.list.version += 1;
        true
    }

    pub fn pull_verify_comment(&mut self) {
        if self.pulls.list.unverified.is_none() {
            return;
        }
        self.pulls.list.verifying = true;
        self.pulls.list.send_error = None;
        self.reload_pull_comments();
    }
}

pub struct NumberedLine {
    pub text: String,
    pub anchor: Option<(u64, Side)>,
}

pub fn numbered_lines(patch: &str) -> Vec<NumberedLine> {
    let mut old = 0u64;
    let mut new = 0u64;
    let mut out = Vec::new();
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            let mut parts = rest.split_whitespace();
            let start = |token: Option<&str>| {
                token
                    .and_then(|t| t.get(1..))
                    .and_then(|t| t.split(',').next())
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(1)
            };
            old = start(parts.next());
            new = start(parts.next());
            out.push(NumberedLine {
                text: line.to_owned(),
                anchor: None,
            });
            continue;
        }
        let anchor = match line.chars().next() {
            Some('+') => {
                new += 1;
                Some((new - 1, Side::Right))
            }
            Some('-') => {
                old += 1;
                Some((old - 1, Side::Left))
            }
            Some('\\') => None,
            _ => {
                old += 1;
                new += 1;
                Some((new - 1, Side::Right))
            }
        };
        out.push(NumberedLine {
            text: line.to_owned(),
            anchor,
        });
    }
    out
}

pub fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
