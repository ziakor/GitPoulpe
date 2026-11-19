use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use gitpoulpe_hosting::{
    DraftLine, LineAnchor, PullComments, PullDetail, PullFilter, PullSummary, Review,
    ReviewDecision, ReviewSubmission, Side, list_comments, list_pulls, list_reviews, load_pull,
    post_comment, post_line_comment, reply_to_comment, review_exists, submit_review,
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

#[derive(Clone, Debug)]
pub struct DraftEntry {
    pub line: DraftLine,
    pub patch_hash: u64,
    pub to_reread: bool,
}

#[derive(Clone, Debug)]
pub struct ReviewDraft {
    pub head_sha: String,
    pub stale: bool,
    pub entries: Vec<DraftEntry>,
    pub body: String,
    pub decision: ReviewDecision,
}

fn patch_hash(detail: &PullDetail, path: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match detail.files.iter().find(|f| f.change.path == path) {
        Some(file) => file.patch.hash(&mut hasher),
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

pub struct PullListState {
    pub drafts: HashMap<String, ReviewDraft>,
    pub review_body: TextEdit,
    pub decision: ReviewDecision,
    pub review_request: u64,
    pub review_sending: bool,
    pub review_error: Option<String>,
    pub review_notice: Option<String>,
    pub review_unverified: bool,
    pub review_verifying: bool,
    pub reviews: Vec<Review>,
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
            drafts: HashMap::new(),
            review_body: TextEdit::new(""),
            decision: ReviewDecision::Comment,
            review_request: 0,
            review_sending: false,
            review_error: None,
            review_notice: None,
            review_unverified: false,
            review_verifying: false,
            reviews: Vec::new(),
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
        let review_request = self.pulls.list.review_request + 1;
        let drafts = std::mem::take(&mut self.pulls.list.drafts);
        self.pulls.list = PullListState {
            drafts,
            review_request,
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
        list.review_request += 1;
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
        if previous != Some(number) {
            self.save_review_text();
        }
        self.pulls.list.selected = Some(number);
        if previous != Some(number) {
            let list = &mut self.pulls.list;
            list.review_request += 1;
            list.review_sending = false;
            list.review_error = None;
            list.review_notice = None;
            list.review_unverified = false;
            list.review_verifying = false;
            list.reviews = Vec::new();
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
            self.load_review_text();
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
        self.reload_reviews(false);
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
        let key = self.review_key().unwrap_or_default();
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
                let fresh = *detail;
                if let Some(draft) = list
                    .drafts
                    .get_mut(&key)
                    .filter(|d| d.head_sha != fresh.summary.head_sha)
                {
                    draft.stale = true;
                    for entry in &mut draft.entries {
                        if patch_hash(&fresh, &entry.line.path) != entry.patch_hash {
                            entry.to_reread = true;
                        }
                    }
                }
                list.detail = Some(fresh);
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

impl State {
    pub fn review_key(&self) -> Option<String> {
        let (connection, owner, name) = self.pull_list_target()?;
        let number = self.pulls.list.selected?;
        Some(format!("{}|{owner}/{name}#{number}", connection.id))
    }

    fn save_review_text(&mut self) {
        let Some(key) = self.review_key() else {
            return;
        };
        let text = self.pulls.list.review_body.text.clone();
        let decision = self.pulls.list.decision;
        match self.pulls.list.drafts.get_mut(&key) {
            Some(draft) => {
                draft.body = text;
                draft.decision = decision;
            }
            None if !text.trim().is_empty() => {
                let head = self
                    .pulls
                    .list
                    .detail
                    .as_ref()
                    .map(|d| d.summary.head_sha.clone())
                    .unwrap_or_default();
                self.pulls.list.drafts.insert(
                    key,
                    ReviewDraft {
                        head_sha: head,
                        stale: false,
                        entries: Vec::new(),
                        body: text,
                        decision,
                    },
                );
            }
            None => {}
        }
    }

    fn load_review_text(&mut self) {
        let draft = self
            .review_key()
            .and_then(|k| self.pulls.list.drafts.get(&k).cloned());
        let list = &mut self.pulls.list;
        match draft {
            Some(d) => {
                list.review_body = TextEdit::new(&d.body);
                list.decision = d.decision;
            }
            None => {
                list.review_body = TextEdit::new("");
                list.decision = ReviewDecision::Comment;
            }
        }
    }

    pub fn current_draft(&self) -> Option<&ReviewDraft> {
        self.pulls.list.drafts.get(&self.review_key()?)
    }

    pub fn pull_add_to_draft(&mut self) {
        let Some(key) = self.review_key() else {
            return;
        };
        let list = &mut self.pulls.list;
        let (Some(pick), Some(detail)) = (list.pick.clone(), list.detail.as_ref()) else {
            return;
        };
        let body = list.compose.text.trim().to_owned();
        if body.is_empty() {
            list.send_error =
                Some("Écrivez le commentaire avant de l'ajouter à la revue.".to_owned());
            list.version += 1;
            return;
        }
        let head = detail.summary.head_sha.clone();
        let hash = patch_hash(detail, &pick.path);
        let draft = list.drafts.entry(key).or_insert_with(|| ReviewDraft {
            head_sha: head.clone(),
            stale: false,
            entries: Vec::new(),
            body: String::new(),
            decision: ReviewDecision::Comment,
        });
        if draft.head_sha != head {
            draft.stale = true;
            list.send_error = Some(
                "Le brouillon porte sur une autre révision : adoptez la révision actuelle avant d'ajouter des commentaires.".to_owned(),
            );
            list.version += 1;
            return;
        }
        draft.entries.push(DraftEntry {
            line: DraftLine {
                path: pick.path,
                line: pick.line,
                start_line: pick.start,
                side: pick.side,
                body,
            },
            patch_hash: hash,
            to_reread: false,
        });
        list.compose = TextEdit::new("");
        list.pick = None;
        list.range_start = None;
        list.send_error = None;
        list.send_notice = Some("Commentaire ajouté au brouillon de revue.".to_owned());
        list.version += 1;
    }

    pub fn pull_remove_draft_line(&mut self, index: usize) {
        if let Some(key) = self.review_key()
            && let Some(draft) = self.pulls.list.drafts.get_mut(&key)
            && index < draft.entries.len()
        {
            draft.entries.remove(index);
        }
        self.pulls.list.version += 1;
    }

    pub fn pull_mark_read(&mut self, index: usize) {
        if let Some(key) = self.review_key()
            && let Some(entry) = self
                .pulls
                .list
                .drafts
                .get_mut(&key)
                .and_then(|d| d.entries.get_mut(index))
        {
            entry.to_reread = false;
        }
        self.pulls.list.version += 1;
    }

    pub fn pull_adopt_head(&mut self) {
        let Some(key) = self.review_key() else {
            return;
        };
        let Some(head) = self
            .pulls
            .list
            .detail
            .as_ref()
            .map(|d| d.summary.head_sha.clone())
        else {
            return;
        };
        if let Some(draft) = self.pulls.list.drafts.get_mut(&key) {
            draft.head_sha = head;
            draft.stale = false;
        }
        self.pulls.list.version += 1;
    }

    pub fn pull_discard_draft(&mut self) {
        if let Some(key) = self.review_key() {
            self.pulls.list.drafts.remove(&key);
        }
        let list = &mut self.pulls.list;
        list.review_body = TextEdit::new("");
        list.decision = ReviewDecision::Comment;
        list.review_error = None;
        list.review_unverified = false;
        list.review_notice = Some("Brouillon de revue abandonné.".to_owned());
        list.version += 1;
    }

    pub fn pull_set_decision(&mut self, decision: ReviewDecision) {
        self.pulls.list.decision = decision;
        self.pulls.list.version += 1;
    }

    pub fn reload_reviews(&mut self, verify: bool) {
        let Some(number) = self.pulls.list.selected else {
            return;
        };
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        self.pulls.list.review_request += 1;
        let request = self.pulls.list.review_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_reviews(secrets.as_ref(), &connection, &owner, &name, number)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullReviewsLoaded {
                request,
                number,
                verify,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn pull_login(&self) -> String {
        self.pulls
            .candidates
            .get(self.pulls.choice)
            .map(|c| c.connection.login.clone())
            .unwrap_or_default()
    }

    fn pending_submission(&self) -> Option<ReviewSubmission> {
        let list = &self.pulls.list;
        let detail = list.detail.as_ref()?;
        let draft = self.current_draft();
        Some(ReviewSubmission {
            commit_sha: draft
                .map(|d| d.head_sha.clone())
                .unwrap_or_else(|| detail.summary.head_sha.clone()),
            body: list.review_body.text.clone(),
            decision: list.decision,
            lines: draft
                .map(|d| d.entries.iter().map(|e| e.line.clone()).collect())
                .unwrap_or_default(),
        })
    }

    pub fn pull_submit_review(&mut self) {
        self.save_review_text();
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let login = self.pull_login();
        let Some(submission) = self.pending_submission() else {
            return;
        };
        let draft = self.current_draft().cloned();
        let list = &mut self.pulls.list;
        let Some(number) = list.selected else {
            return;
        };
        if list.review_sending {
            return;
        }
        list.review_error = None;
        list.review_notice = None;
        let author = list
            .detail
            .as_ref()
            .map(|d| d.summary.author.clone())
            .unwrap_or_default();
        let refusal = if draft.as_ref().is_some_and(|d| d.stale) {
            Some("La pull request a été mise à jour depuis la préparation de la revue : adoptez la révision actuelle et relisez les commentaires signalés avant d'envoyer.".to_owned())
        } else if let Some(count) = draft
            .as_ref()
            .map(|d| d.entries.iter().filter(|e| e.to_reread).count())
            .filter(|c| *c > 0)
        {
            Some(format!(
                "{count} commentaire(s) du brouillon sont à relire (fichier modifié) : marquez-les relus ou retirez-les."
            ))
        } else if submission.decision != ReviewDecision::Comment
            && author.eq_ignore_ascii_case(&login)
        {
            Some("Vous êtes l'auteur de cette pull request : GitHub n'autorise ni l'approbation ni la demande de changements sur sa propre PR.".to_owned())
        } else {
            None
        };
        if let Some(message) = refusal {
            list.review_error = Some(message);
            list.version += 1;
            return;
        }
        list.review_request += 1;
        list.review_sending = true;
        list.review_unverified = false;
        list.version += 1;
        let request = list.review_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = submit_review(
                secrets.as_ref(),
                &connection,
                &owner,
                &name,
                number,
                &submission,
            )
            .await;
            let unknown = matches!(
                result,
                Err(gitpoulpe_hosting::GithubError::Http(_)
                    | gitpoulpe_hosting::GithubError::Server(_))
            );
            let _ = sender.send(Event::PullReviewSent {
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

    pub fn apply_pull_review_sent(
        &mut self,
        request: u64,
        number: u64,
        unknown: bool,
        result: Result<Review, String>,
    ) -> bool {
        let key = self.review_key();
        let login = self.pull_login();
        let list = &mut self.pulls.list;
        if !list.open || request != list.review_request || list.selected != Some(number) {
            return false;
        }
        list.review_sending = false;
        match result {
            Ok(review) => {
                if let Some(key) = key {
                    list.drafts.remove(&key);
                }
                list.review_body = TextEdit::new("");
                list.decision = ReviewDecision::Comment;
                list.review_notice = Some(format!(
                    "Revue publiée par {login} : {} sur la révision {}.",
                    review_state_label(&review.state),
                    short(&review.commit_sha)
                ));
                list.reviews.push(review);
            }
            Err(message) => {
                list.review_error = Some(message);
                list.review_unverified = unknown;
            }
        }
        list.version += 1;
        true
    }

    pub fn pull_verify_review(&mut self) {
        if !self.pulls.list.review_unverified {
            return;
        }
        self.pulls.list.review_verifying = true;
        self.pulls.list.review_error = None;
        self.reload_reviews(true);
    }

    pub fn apply_pull_reviews(
        &mut self,
        request: u64,
        number: u64,
        verify: bool,
        result: Result<Vec<Review>, String>,
    ) -> bool {
        let key = self.review_key();
        let login = self.pull_login();
        let submission = self.pending_submission();
        let list = &mut self.pulls.list;
        if !list.open || request != list.review_request || list.selected != Some(number) {
            return false;
        }
        match result {
            Ok(reviews) => {
                if verify && list.review_verifying {
                    list.review_verifying = false;
                    let sent = submission
                        .as_ref()
                        .is_some_and(|s| review_exists(&reviews, &login, s));
                    if sent {
                        if let Some(key) = key {
                            list.drafts.remove(&key);
                        }
                        list.review_body = TextEdit::new("");
                        list.review_unverified = false;
                        list.review_notice =
                            Some("La revue est déjà publiée : ne la renvoyez pas.".to_owned());
                    } else {
                        list.review_unverified = false;
                        list.review_notice = Some(
                            "La revue n'a pas été publiée : votre brouillon est intact, vous pouvez réessayer.".to_owned(),
                        );
                    }
                }
                list.reviews = reviews;
            }
            Err(message) => {
                list.review_verifying = false;
                list.review_error = Some(message);
            }
        }
        list.version += 1;
        true
    }
}

pub fn review_state_label(state: &str) -> &'static str {
    match state {
        "APPROVED" => "approbation",
        "CHANGES_REQUESTED" => "changements demandés",
        "COMMENTED" => "commentaire",
        "DISMISSED" => "écartée",
        "PENDING" => "brouillon",
        _ => "revue",
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
