use std::borrow::Cow;

use gitpoulpe_hosting::{PullFilter, PullState, PullSummary};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::pull_list::short;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

const ROW_HEIGHT: f32 = 52.0;
const MAX_PATCH_LINES: usize = 3000;

fn state_label(pull: &PullSummary) -> &'static str {
    match (pull.state, pull.draft) {
        (PullState::Merged, _) => "fusionnée",
        (PullState::Closed, _) => "fermée",
        (PullState::Open, true) => "brouillon",
        (PullState::Open, false) => "ouverte",
    }
}

fn pull_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(pull) = s.pulls.list.items.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let number = pull.number;
    let mut item = el()
        .key(format!("prl-row{index}"))
        .role("option")
        .label(format!("#{} {}", pull.number, pull.title))
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .on_press(move |s: &mut State| s.select_pull(number));
    if s.pulls.list.selected == Some(pull.number) {
        item = item.fill(p.panel);
    }
    item.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(
                text(format!("#{} {}", pull.number, pull.title))
                    .font(theme::BODY)
                    .color(p.text)
                    .truncate(),
            )
            .child(
                text(format!("{} · {}", state_label(pull), pull.author))
                    .font(theme::SMALL)
                    .color(p.muted)
                    .truncate(),
            ),
    )
}

fn filter_button(s: &State, key: &str, caption: &str, filter: PullFilter) -> El {
    action(s, key, caption, s.pulls.list.filter == filter)
        .on_press(move |s: &mut State| s.set_pull_filter(filter))
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut panel = col()
        .key("prl-panel")
        .w_full()
        .h_full()
        .grow(1.0)
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(error) = &list.detail_error {
        panel = panel.child(
            text(error.clone())
                .key("prl-detail-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if list.detail_loading {
        panel = panel.child(
            text("Chargement de la pull request…".to_owned())
                .key("prl-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(notice) = &list.notice {
        panel = panel.child(
            text(notice.clone())
                .key("prl-notice")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let Some(detail) = &list.detail else {
        if list.selected.is_none() {
            panel = panel.child(
                text("Sélectionnez une pull request.".to_owned())
                    .key("prl-hint")
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        return panel;
    };
    let pull = &detail.summary;
    panel = panel
        .child(
            text(format!("#{} {}", pull.number, pull.title))
                .key("prl-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!(
                "{} · par {} · mise à jour {}",
                state_label(pull),
                pull.author,
                pull.updated_at
            ))
            .key("prl-meta")
            .font(theme::SMALL)
            .color(p.muted)
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
            .color(p.text)
            .wrap(),
        );
    if !detail.body.trim().is_empty() {
        panel = panel.child(
            text(detail.body.clone())
                .key("prl-body")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    panel = panel.child(
        text(format!("{} commit(s)", detail.commits.len()))
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    for (i, commit) in detail.commits.iter().enumerate() {
        panel = panel.child(
            text(format!(
                "{} {} — {}",
                short(&commit.sha),
                commit.subject,
                commit.author
            ))
            .key(format!("prl-commit{i}"))
            .font(theme::PATH_SMALL)
            .color(p.muted)
            .truncate(),
        );
    }
    panel = panel.child(
        text(format!(
            "{} fichier(s){}",
            detail.files.len(),
            if detail.files_truncated {
                " (liste tronquée)"
            } else {
                ""
            }
        ))
        .key("prl-files-title")
        .font(theme::BODY_STRONG)
        .color(p.text),
    );
    let mut budget = MAX_PATCH_LINES;
    for (i, file) in detail.files.iter().enumerate() {
        panel = panel.child(
            text(format!(
                "{} {} (+{} −{})",
                file.change.status, file.change.path, file.change.additions, file.change.deletions
            ))
            .key(format!("prl-file{i}"))
            .font(theme::PATH)
            .color(p.text)
            .truncate(),
        );
        match &file.patch {
            None => {
                panel = panel.child(
                    text(
                        "Pas de diff texte fourni pour ce fichier (binaire ou trop volumineux)."
                            .to_owned(),
                    )
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
                );
            }
            Some(patch) => {
                for (n, line) in patch.lines().enumerate() {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    let mut line_el = row().w_full().px(6.0).child(
                        text(line.to_owned())
                            .key(format!("prl-f{i}l{n}"))
                            .font(theme::CODE)
                            .color(p.text),
                    );
                    if line.starts_with('+') {
                        line_el = line_el.fill(p.added);
                    } else if line.starts_with('-') {
                        line_el = line_el.fill(p.removed);
                    }
                    panel = panel.child(line_el);
                }
            }
        }
    }
    if budget == 0 {
        panel = panel.child(
            text(format!(
                "Affichage limité à {MAX_PATCH_LINES} lignes de diff ; le reste n'est pas affiché."
            ))
            .key("prl-patch-limit")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    panel
}

pub fn pull_list_screen(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(8.0)
        .child(
            text(format!("Pull requests · {} résultat(s)", list.items.len()))
                .key("prl-title-line")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(filter_button(s, "prl-open", "Ouvertes", PullFilter::Open))
        .child(filter_button(
            s,
            "prl-closed",
            "Fermées",
            PullFilter::Closed,
        ))
        .child(filter_button(s, "prl-all", "Toutes", PullFilter::All))
        .child(
            action(s, "prl-refresh", "Actualiser", false).on_press(|s: &mut State| {
                s.reload_pull_list();
                s.load_pull_detail();
            }),
        )
        .child(
            action(s, "prl-back", "Retour", false).on_press(|s: &mut State| s.close_pull_list()),
        );
    let mut left = col().w(380.0).h_full().border(1.0, p.line);
    if let Some(error) = &list.error {
        left = left.child(
            text(error.clone())
                .key("prl-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap()
                .px(16.0)
                .py(8.0),
        );
    } else if list.loading {
        left = left.child(
            text("Chargement…".to_owned())
                .key("prl-list-loading")
                .font(theme::SMALL)
                .color(p.muted)
                .px(16.0)
                .py(8.0),
        );
    } else if list.items.is_empty() {
        left = left.child(
            text("Aucune pull request pour ce filtre.".to_owned())
                .key("prl-empty")
                .font(theme::SMALL)
                .color(p.muted)
                .px(16.0)
                .py(8.0),
        );
    }
    if list.truncated {
        left = left.child(
            text("Liste tronquée : trop de pull requests pour tout afficher.".to_owned())
                .key("prl-truncated")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap()
                .px(16.0)
                .py(4.0),
        );
    }
    left = left.child(
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
    );
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(left)
            .child(detail(s)),
    )
}
