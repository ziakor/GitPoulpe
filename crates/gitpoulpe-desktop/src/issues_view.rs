use std::borrow::Cow;

use gitpoulpe_hosting::{IssueFilter, IssueLink};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, screen_head};

const ROW_HEIGHT: f32 = 52.0;

fn issue_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(issue) = s.issues.items.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let number = issue.number;
    let mut item = el()
        .key(format!("is-row{index}"))
        .role("option")
        .label(format!("#{} {}", issue.number, issue.title))
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .on_press(move |s: &mut State| s.select_issue(number));
    if s.issues.selected == Some(issue.number) {
        item = item.fill(p.panel);
    }
    item.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(
                text(format!("#{} {}", issue.number, issue.title))
                    .font(theme::BODY)
                    .color(p.text)
                    .truncate(),
            )
            .child(
                text(format!(
                    "{} · {} · {} commentaire(s){}",
                    if issue.open { "ouverte" } else { "fermée" },
                    issue.author,
                    issue.comments,
                    if issue.labels.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", issue.labels.join(", "))
                    }
                ))
                .font(theme::SMALL)
                .color(p.muted)
                .truncate(),
            ),
    )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let issues = &s.issues;
    let mut panel = col()
        .key("is-panel")
        .w_full()
        .h_full()
        .grow(1.0)
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(error) = &issues.detail_error {
        return panel.child(
            text(error.clone())
                .key("is-detail-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if issues.detail_loading {
        return panel.child(
            text("Chargement de l'issue…".to_owned())
                .key("is-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    let Some(detail) = &issues.detail else {
        return panel.child(
            text("Sélectionnez une issue.".to_owned())
                .key("is-hint")
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    let summary = &detail.summary;
    panel = panel
        .child(
            text(format!("#{} {}", summary.number, summary.title))
                .key("is-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!(
                "{} · par {} · mise à jour {}{}",
                if summary.open { "ouverte" } else { "fermée" },
                summary.author,
                summary.updated_at,
                if summary.labels.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", summary.labels.join(", "))
                }
            ))
            .key("is-meta")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    if let Some(instance) = &detail.instance {
        panel = panel.child(
            text(format!("Serveur : {instance}"))
                .key("is-instance")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    for (index, limit) in detail.limits.iter().enumerate() {
        panel = panel.child(
            text(format!("Limite : {limit}."))
                .key(format!("is-limit{index}"))
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &issues.notice {
        panel = panel.child(
            text(notice.clone())
                .key("is-notice")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if !detail.body.trim().is_empty() {
        panel = panel.child(
            text(detail.body.clone())
                .key("is-body")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    for (index, link) in detail.links.iter().enumerate() {
        panel = match link {
            IssueLink::Pull {
                number,
                title,
                open,
            } => {
                let number = *number;
                panel.child(
                    row()
                        .key(format!("is-link{index}"))
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(format!(
                                "#{number} {title} · {}",
                                if *open {
                                    "ouverte"
                                } else {
                                    "fermée ou fusionnée"
                                }
                            ))
                            .font(theme::SMALL)
                            .color(p.text)
                            .grow(1.0)
                            .truncate(),
                        )
                        .child(
                            action(
                                s,
                                &format!("is-pull{index}"),
                                "Ouvrir la demande de fusion",
                                false,
                            )
                            .on_press(move |s: &mut State| s.issue_open_pull(number)),
                        ),
                )
            }
            IssueLink::Commit { sha } => {
                let sha = sha.clone();
                panel.child(
                    row()
                        .key(format!("is-link{index}"))
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(format!(
                                "commit {}",
                                sha.chars().take(10).collect::<String>()
                            ))
                            .font(theme::PATH_SMALL)
                            .color(p.text)
                            .grow(1.0),
                        )
                        .child(
                            action(s, &format!("is-commit{index}"), "Voir le commit", false)
                                .on_press(move |s: &mut State| s.issue_open_commit(&sha)),
                        ),
                )
            }
        };
    }
    if !issues.branches.is_empty() {
        panel = panel.child(
            text(format!(
                "Branches locales liées : {}",
                issues.branches.join(", ")
            ))
            .key("is-branches")
            .font(theme::SMALL)
            .color(p.text)
            .wrap(),
        );
    }
    panel = panel.child(
        text(format!("{} commentaire(s)", detail.notes.len()))
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    for (index, note) in detail.notes.iter().enumerate() {
        panel = panel.child(
            text(format!("{} : {}", note.author, note.body))
                .key(format!("is-note{index}"))
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if detail.notes_truncated {
        panel = panel.child(
            text("Liste de commentaires tronquée : le reste n'est pas affiché.".to_owned())
                .key("is-notes-cut")
                .font(theme::SMALL)
                .color(p.red),
        );
    }
    panel
}

fn filter_button(s: &State, key: &str, caption: &str, filter: IssueFilter) -> El {
    action(s, key, caption, s.issues.filter == filter)
        .on_press(move |s: &mut State| s.issues_set_filter(filter))
}

pub fn issues_screen(s: &State) -> El {
    let p = s.palette;
    let issues = &s.issues;
    let who = issues
        .targets
        .get(issues.choice)
        .map(|t| {
            format!(
                "{}/{} · {}@{}",
                t.owner,
                t.name,
                t.connection.login,
                t.connection.host_caption()
            )
        })
        .unwrap_or_default();
    let mut actions = vec![
        filter_button(s, "is-open", "Ouvertes", IssueFilter::Open),
        filter_button(s, "is-closed", "Fermées", IssueFilter::Closed),
        filter_button(s, "is-all", "Toutes", IssueFilter::All),
    ];
    if issues.targets.len() > 1 {
        actions.push(
            action(s, "is-account", "Changer de compte", false)
                .on_press(|s: &mut State| s.issues_switch_target()),
        );
    }
    let header = screen_head(
        s,
        "is-head",
        "Issues",
        format!(
            "{who} · {}",
            counted(issues.items.len(), "résultat", "résultats")
        ),
        actions,
    );
    let search = row()
        .w_full()
        .px(20.0)
        .gap(8.0)
        .child(
            text_input(&issues.query, theme::BODY)
                .key("is-query")
                .label("Rechercher dans les issues")
                .single_line()
                .grow(1.0)
                .h(34.0)
                .px(10.0)
                .py(8.0)
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .color(p.text)
                .placeholder("Rechercher dans les issues…".to_owned(), p.muted)
                .focus_ring(2.0, 2.0, p.accent)
                .on_edit(|s: &mut State| &mut s.issues.query)
                .on_input_key(|s: &mut State, event, _| {
                    if event.key == rune::Key::Enter {
                        s.reload_issues();
                        return true;
                    }
                    false
                }),
        )
        .child(
            action(s, "is-search", "Rechercher", true).on_press(|s: &mut State| s.reload_issues()),
        );
    let list: El = if let Some(error) = &issues.error {
        col().w(420.0).h_full().px(16.0).py(14.0).child(
            text(error.clone())
                .key("is-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        )
    } else if issues.items.is_empty() && !issues.loading {
        col().w(420.0).h_full().px(16.0).py(14.0).child(
            text("Aucune issue pour ce filtre.".to_owned())
                .key("is-empty")
                .font(theme::SMALL)
                .color(p.muted),
        )
    } else {
        col()
            .key("is-viewport")
            .w(420.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("is-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((issues.version, s.dark))
                    .virtual_rows(
                        issues.items.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("isk{i}")),
                        issue_row,
                    ),
            )
    };
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(search);
    if issues.truncated {
        body = body.child(
            col().w_full().px(20.0).py(4.0).child(
                text(
                    "Liste tronquée : trop d'issues pour tout afficher, affinez la recherche."
                        .to_owned(),
                )
                .key("is-truncated")
                .font(theme::SMALL)
                .color(p.red),
            ),
        );
    }
    body.child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(detail(s)),
    )
}
