use std::borrow::Cow;

use gitpoulpe_hosting::{PullDetail, PullFilter, PullState, PullSummary};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::attachments_view::{Body, rich_body};
use crate::pull_list::{CommentKind, ReviewTab, short, when};
use crate::review_files_view::{files_tab, send_status};
use crate::review_side_view::side_sections;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, area, quiet, text_tab};

const ROW_HEIGHT: f32 = 88.0;
const LIST_WIDTH: f32 = 300.0;
const LIST_WIDTH_NARROW: f32 = 252.0;
const SIDE_WIDTH: f32 = 320.0;
const WIDE: f32 = 1180.0;
const NARROW: f32 = 960.0;
const MEASURE: f32 = 780.0;

pub fn state_word(pull: &PullSummary) -> &'static str {
    match (pull.state, pull.draft) {
        (PullState::Merged, _) => "fusionnée",
        (PullState::Closed, _) => "fermée",
        (PullState::Open, true) => "brouillon",
        (PullState::Open, false) => "ouverte",
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn wide(s: &State) -> bool {
    s.window_width <= 0.0 || s.window_width >= WIDE
}

fn review_verb(state: &str) -> &'static str {
    match state {
        "APPROVED" => "a approuvé",
        "CHANGES_REQUESTED" => "a demandé des changements sur",
        "DISMISSED" => "revue écartée sur",
        "PENDING" => "revue en brouillon sur",
        _ => "a laissé une revue sur",
    }
}

fn pull_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(pull) = s.pulls.list.items.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let number = pull.number;
    let selected = s.pulls.list.selected == Some(number);
    let rule = if selected {
        el().w(2.0).h_full().fill(p.accent)
    } else {
        el().w(2.0).h_full()
    };
    let mut item = row()
        .key(format!("prl-row{index}"))
        .role("option")
        .label(format!("#{} {}", pull.number, pull.title))
        .focus_selected(selected)
        .w_full()
        .h(ROW_HEIGHT)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_pull(number));
    if selected {
        item = item.fill(p.selected);
    }
    item.child(rule).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .px(14.0)
            .py(10.0)
            .gap(3.0)
            .child(
                text(pull.title.clone())
                    .font(theme::BODY_MEDIUM)
                    .color(p.text)
                    .w_full()
                    .max_lines(2),
            )
            .child(
                text(format!(
                    "#{} · {} · {}",
                    pull.number,
                    state_word(pull),
                    pull.author
                ))
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .truncate(),
            )
            .child(
                text(format!("{} → {}", pull.head_ref, pull.base_ref))
                    .font(theme::PATH_SMALL)
                    .color(p.muted)
                    .w_full()
                    .truncate(),
            ),
    )
}

fn filter_tab(s: &State, key: &str, caption: &str, filter: PullFilter) -> El {
    text_tab(s, key, caption.to_owned(), s.pulls.list.filter == filter)
        .on_press(move |s: &mut State| s.set_pull_filter(filter))
}

fn screen_header(s: &State) -> El {
    let p = s.palette;
    let place = s
        .pull_list_target()
        .map(|(connection, owner, name)| format!("{} · {owner}/{name}", connection.host_caption()))
        .unwrap_or_default();
    row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(8.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(if s.pull_is_gitlab() {
                        "Merge requests"
                    } else {
                        "Pull requests"
                    })
                    .key("prl-title-line")
                    .font(theme::HEADING)
                    .color(p.text)
                    .w_full()
                    .truncate(),
                )
                .child(
                    text(place)
                        .key("prl-place")
                        .font(theme::SMALL)
                        .color(p.muted)
                        .w_full()
                        .truncate(),
                ),
        )
        .child(
            action(s, "prl-refresh", "Actualiser", false).on_press(|s: &mut State| {
                s.reload_pull_list();
                s.load_pull_detail();
            }),
        )
        .child(action(s, "prl-back", "Retour", false).on_press(|s: &mut State| s.close_pull_list()))
}

fn list_column(s: &State, width: f32) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let filters = row()
        .w_full()
        .px(16.0)
        .gap(18.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .child(filter_tab(s, "prl-open", "Ouvertes", PullFilter::Open))
        .child(filter_tab(s, "prl-closed", "Fermées", PullFilter::Closed))
        .child(filter_tab(s, "prl-all", "Toutes", PullFilter::All))
        .child(el().grow(1.0))
        .child(
            text(list.items.len().to_string())
                .key("prl-count")
                .font(theme::SMALL)
                .color(p.muted),
        );
    let message = |key: &str, words: String, red: bool| {
        text(words)
            .key(key.to_owned())
            .font(theme::SMALL)
            .color(if red { p.red } else { p.muted })
            .w_full()
            .wrap()
            .px(16.0)
            .py(10.0)
    };
    let mut column = col()
        .w(width)
        .h_full()
        .fill(p.panel)
        .border_sides([0.0, 1.0, 0.0, 0.0], p.line)
        .child(filters);
    if let Some(error) = &list.error {
        column = column.child(message("prl-error", error.clone(), true));
    } else if list.loading {
        column = column.child(message("prl-list-loading", "Chargement…".to_owned(), false));
    } else if list.items.is_empty() {
        column = column.child(message(
            "prl-empty",
            format!("Aucune {} pour ce filtre.", s.pull_noun()),
            false,
        ));
    }
    if list.truncated {
        column = column.child(message(
            "prl-truncated",
            format!(
                "Liste tronquée : trop de {}s pour tout afficher.",
                s.pull_noun()
            ),
            false,
        ));
    }
    column.child(
        col()
            .key("prl-viewport")
            .grow(1.0)
            .min_w(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("prl-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((list.version, s.dark))
                    .virtual_rows(
                        list.items.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("prlk{i}")),
                        pull_row,
                    ),
            ),
    )
}

fn detail_header(s: &State, detail: &PullDetail) -> El {
    let p = s.palette;
    let pull = &detail.summary;
    let mut actions = row().gap(14.0).pt(2.0);
    if !pull.url.is_empty() {
        let url = pull.url.clone();
        actions = actions.child(
            quiet(s, "prl-web", "Ouvrir dans le navigateur")
                .on_press_tx(move |s: &mut State, tx| s.open_link(tx, &url)),
        );
    }
    actions = actions.child(
        quiet(s, "prl-git", "Lire toute la PR depuis Git")
            .on_press(|s: &mut State| s.open_pull_from_git()),
    );
    col()
        .w_full()
        .px(24.0)
        .pt(16.0)
        .pb(12.0)
        .gap(4.0)
        .child(
            text(pull.title.clone())
                .key("prl-title")
                .font(theme::HEADING)
                .color(p.text)
                .w_full()
                .max_lines(2),
        )
        .child(
            text(format!(
                "#{} · {} · par {} · mise à jour le {}",
                pull.number,
                capitalized(state_word(pull)),
                pull.author,
                when(&pull.updated_at)
            ))
            .key("prl-meta")
            .font(theme::SMALL)
            .color(p.muted)
            .w_full()
            .wrap(),
        )
        .child(
            text(format!(
                "{} ({}) → {} ({})",
                pull.head_ref,
                short(&pull.head_sha),
                pull.base_ref,
                short(&pull.base_sha)
            ))
            .key("prl-revisions")
            .font(theme::PATH_SMALL)
            .color(p.muted)
            .w_full()
            .truncate(),
        )
        .child(actions)
}

fn notes(s: &State, detail: &PullDetail) -> Option<El> {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut lines: Vec<(&str, String)> = Vec::new();
    if let Some(error) = &list.detail_error {
        lines.push(("prl-detail-error", error.clone()));
    }
    if let Some(notice) = &list.notice {
        lines.push(("prl-notice", notice.clone()));
    }
    if s.pull_is_gitlab() && detail.files_truncated {
        lines.push(("prl-cap", "GitLab n'a pas renvoyé la liste complète des fichiers (limite de l'API) : utilisez « Lire depuis Git » pour tous les fichiers.".to_owned()));
    } else if !s.pull_is_gitlab() && (detail.files_truncated || detail.files.len() >= 3000) {
        lines.push(("prl-cap", "L'API GitHub plafonne la liste à 3000 fichiers : utilisez « Lire depuis Git » pour tous les fichiers.".to_owned()));
    }
    if list.detail_loading {
        lines.push((
            "prl-loading",
            format!("Actualisation de la {}…", s.pull_noun()),
        ));
    }
    if lines.is_empty() {
        return None;
    }
    let mut column = col().w_full().px(24.0).pb(10.0).gap(4.0);
    for (key, words) in lines {
        let calm = key == "prl-loading";
        column = column.child(
            text(words)
                .key(key)
                .font(theme::SMALL)
                .color(if calm { p.muted } else { p.red })
                .w_full()
                .wrap(),
        );
    }
    Some(column)
}

fn tabs(s: &State, detail: &PullDetail, wide: bool, current: ReviewTab) -> El {
    let p = s.palette;
    let tab = |key: &str, caption: String, which: ReviewTab| {
        text_tab(s, key, caption, current == which)
            .on_press(move |s: &mut State| s.set_review_tab(which))
    };
    let mut bar = row()
        .key("prl-tabs")
        .role("tablist")
        .w_full()
        .px(24.0)
        .gap(22.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .child(tab(
            "prl-tab-conversation",
            "Conversation".to_owned(),
            ReviewTab::Conversation,
        ))
        .child(tab(
            "prl-tab-files",
            format!("Fichiers {}", detail.files.len()),
            ReviewTab::Files,
        ))
        .child(tab(
            "prl-tab-commits",
            format!("Commits {}", detail.commits.len()),
            ReviewTab::Commits,
        ));
    if !wide {
        bar = bar.child(tab(
            "prl-tab-decision",
            "Revue et fusion".to_owned(),
            ReviewTab::Decision,
        ));
    }
    bar
}

fn entry_head(s: &State, author: &str, verb: &str, stamp: &str) -> El {
    let p = s.palette;
    let mut head = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(
            text(author.to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(text(verb.to_owned()).font(theme::SMALL).color(p.muted));
    if !stamp.is_empty() {
        head = head.child(
            text(format!("· {}", when(stamp)))
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    head
}

fn entry(s: &State, key: String, head: El, body: El, first: bool) -> El {
    let p = s.palette;
    col()
        .key(key)
        .w_full()
        .py(14.0)
        .gap(8.0)
        .border_sides([if first { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0], p.line)
        .child(head)
        .child(body)
}

fn conversation(s: &State, detail: &PullDetail) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let description = if detail.body.trim().is_empty() {
        text("Pas de description.".to_owned())
            .key("prl-body")
            .font(theme::BODY)
            .color(p.muted)
    } else {
        rich_body(
            s,
            Body {
                key: "prl-body".to_owned(),
                prefix: String::new(),
                text: &detail.body,
                suffix: String::new(),
                attachments: &detail.attachments,
                style: theme::BODY,
                color: p.text,
            },
        )
    };
    let mut column = col().w_full().max_w(MEASURE).child(entry(
        s,
        "prl-description".to_owned(),
        entry_head(
            s,
            &detail.summary.author,
            &format!("a ouvert la {}", s.pull_noun()),
            "",
        ),
        description,
        true,
    ));
    let mut timeline: Vec<(&str, El)> = Vec::new();
    for (i, comment) in list.comments.issue.iter().enumerate() {
        let body = rich_body(
            s,
            Body {
                key: format!("prc-general{i}"),
                prefix: String::new(),
                text: &comment.body,
                suffix: String::new(),
                attachments: &comment.attachments,
                style: theme::BODY,
                color: p.text,
            },
        );
        timeline.push((
            comment.created_at.as_str(),
            entry(
                s,
                format!("prc-entry{i}"),
                entry_head(s, &comment.author, "a commenté", &comment.created_at),
                body,
                false,
            ),
        ));
    }
    for (i, review) in list.reviews.iter().enumerate() {
        let verb = format!(
            "{} la révision {}",
            review_verb(&review.state),
            short(&review.commit_sha)
        );
        let body = if review.body.trim().is_empty() {
            el().h(0.0)
        } else {
            text(review.body.clone())
                .key(format!("prv-published{i}-body"))
                .font(theme::BODY)
                .color(p.text)
                .w_full()
                .wrap()
        };
        timeline.push((
            review.submitted_at.as_str(),
            entry(
                s,
                format!("prv-published{i}"),
                entry_head(s, &review.author, &verb, &review.submitted_at),
                body,
                false,
            ),
        ));
    }
    timeline.sort_by(|a, b| a.0.cmp(b.0));
    for (_, item) in timeline {
        column = column.child(item);
    }
    column.child(general_composer(s))
}

fn general_composer(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let block = col()
        .key("prc-general-box")
        .w_full()
        .pt(16.0)
        .pb(24.0)
        .gap(8.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], p.line)
        .child(
            text("Ajouter un commentaire".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        );
    if list.pick.is_some() || list.reply_to.is_some() {
        return block
            .child(
                text("Un commentaire de ligne est en cours dans l'onglet Fichiers.".to_owned())
                    .key("prc-elsewhere")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .w_full()
                    .wrap(),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(
                        quiet(s, "prc-goto-files", "Y aller")
                            .on_press(|s: &mut State| s.set_review_tab(ReviewTab::Files)),
                    )
                    .child(
                        quiet(s, "prc-cancel", "Abandonner ce commentaire")
                            .on_press(|s: &mut State| s.pull_clear_target()),
                    ),
            );
    }
    block
        .child(
            area(
                s,
                "prc-body",
                "Commentaire",
                &list.compose,
                "Votre commentaire sur l'ensemble de la pull request",
            )
            .on_edit(|s: &mut State| &mut s.pulls.list.compose),
        )
        .child(
            row().child(
                action(
                    s,
                    "prc-send-general",
                    if list.sending {
                        "Envoi…"
                    } else {
                        "Commenter"
                    },
                    true,
                )
                .on_press(|s: &mut State| s.pull_send_comment(CommentKind::General)),
            ),
        )
        .child(send_status(s))
}

fn commits(s: &State, detail: &PullDetail) -> El {
    let p = s.palette;
    let mut column = col().key("prl-commits").w_full().max_w(MEASURE);
    if detail.commits.is_empty() {
        return column.child(
            text("Aucun commit.".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    for (i, commit) in detail.commits.iter().enumerate() {
        column = column.child(
            row()
                .key(format!("prl-commit{i}"))
                .w_full()
                .py(9.0)
                .gap(14.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
                .child(
                    text(short(&commit.sha).to_owned())
                        .font(theme::PATH_SMALL)
                        .color(p.muted)
                        .w(60.0),
                )
                .child(
                    text(commit.subject.clone())
                        .font(theme::BODY)
                        .color(p.text)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    text(commit.author.clone())
                        .font(theme::SMALL)
                        .color(p.muted),
                ),
        );
    }
    column
}

fn empty_detail(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut panel = col()
        .key("prl-panel")
        .w_full()
        .grow(1.0)
        .px(24.0)
        .py(20.0)
        .gap(6.0);
    if let Some(error) = &list.detail_error {
        panel = panel.child(
            text(error.clone())
                .key("prl-detail-error")
                .font(theme::SMALL)
                .color(p.red)
                .w_full()
                .wrap(),
        );
    }
    if let Some(notice) = &list.notice {
        panel = panel.child(
            text(notice.clone())
                .key("prl-notice")
                .font(theme::SMALL)
                .color(p.red)
                .w_full()
                .wrap(),
        );
    }
    if list.detail_loading {
        panel = panel.child(
            text(format!("Chargement de la {}…", s.pull_noun()))
                .key("prl-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    } else if list.selected.is_none() {
        panel = panel.child(
            text(format!("Sélectionnez une {} dans la liste.", s.pull_noun()))
                .key("prl-hint")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    panel
}

fn main_column(s: &State, wide: bool) -> El {
    let list = &s.pulls.list;
    let column = col().grow(1.0).min_w(0.0).h_full();
    let Some(detail) = &list.detail else {
        return column.child(empty_detail(s));
    };
    let current = match list.tab {
        ReviewTab::Decision if wide => ReviewTab::Conversation,
        other => other,
    };
    let content = match current {
        ReviewTab::Conversation => conversation(s, detail),
        ReviewTab::Files => files_tab(s, detail),
        ReviewTab::Commits => commits(s, detail),
        ReviewTab::Decision => side_sections(s),
    };
    let mut column = column.child(detail_header(s, detail));
    if let Some(notes) = notes(s, detail) {
        column = column.child(notes);
    }
    column.child(tabs(s, detail, wide, current)).child(
        col()
            .key("prl-panel")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(24.0)
            .py(8.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(content),
    )
}

fn side_column(s: &State) -> El {
    let p = s.palette;
    col()
        .key("prl-side")
        .w(SIDE_WIDTH)
        .h_full()
        .fill(p.panel)
        .border_sides([0.0, 0.0, 0.0, 1.0], p.line)
        .px(18.0)
        .py(16.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(side_sections(s))
}

pub fn pull_list_screen(s: &State) -> El {
    let wide = wide(s);
    let narrow = s.window_width > 0.0 && s.window_width < NARROW;
    let mut body = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(list_column(
            s,
            if narrow {
                LIST_WIDTH_NARROW
            } else {
                LIST_WIDTH
            },
        ))
        .child(main_column(s, wide));
    if wide && s.pulls.list.detail.is_some() {
        body = body.child(side_column(s));
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(screen_header(s))
        .child(body)
}
