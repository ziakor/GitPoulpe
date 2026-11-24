use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::history_state::format_date;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, field, screen_head};

const ROW_HEIGHT: f32 = 48.0;

fn entry_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(entry) = s.reflog.entries.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.reflog.selected == Some(index);
    let title = format!("{} · {}", entry.selector, entry.message);
    let detail = format!(
        "{} · {} UTC{}",
        entry.oid.chars().take(10).collect::<String>(),
        format_date(entry.timestamp),
        if entry.available {
            ""
        } else {
            " · objet absent"
        }
    );
    el().key(format!("rl-row{index}"))
        .role("option")
        .label(title.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_reflog(index))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(title)
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(if entry.available { p.muted } else { p.red })
                        .truncate(),
                ),
        )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("rl-panel")
        .w(420.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(entry) = s.reflog.selected_entry() else {
        return panel.child(
            text("Sélectionnez une entrée pour l'inspecter ou en récupérer le commit.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel.child(
        text(format!("{} · {}", entry.selector, entry.oid))
            .key("rl-selected")
            .font(theme::PATH_SMALL)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    match &s.reflog.inspected {
        None => {
            panel = panel.child(
                text("Lecture…".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        Some(inspected) if !inspected.available => {
            return panel.child(
                text("Cet objet n'est plus dans le dépôt (expiré ou nettoyé) : il ne peut ni être inspecté ni être récupéré.".to_owned())
                    .key("rl-missing")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
        Some(inspected) => {
            panel = panel
                .child(
                    text(inspected.subject.clone())
                        .key("rl-subject")
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .wrap(),
                )
                .child(
                    text(format!(
                        "{} · {} UTC",
                        inspected.author,
                        format_date(inspected.timestamp)
                    ))
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
                )
                .child(
                    text(if inspected.reachable_from.is_empty() {
                        "Aucune branche ni tag n'atteint plus ce commit : il n'est conservé que par le reflog.".to_owned()
                    } else {
                        format!("Encore atteint par : {}", inspected.reachable_from.join(", "))
                    })
                    .key("rl-reach")
                    .font(theme::SMALL)
                    .color(p.text)
                    .wrap(),
                );
        }
    }
    panel
        .child(
            field(
                s,
                "rl-name",
                "Nom de la branche de récupération",
                &s.reflog.name,
                "recuperation",
            )
            .on_edit(|s: &mut State| &mut s.reflog.name),
        )
        .child(
            action(
                s,
                "rl-recover",
                if s.reflog.busy {
                    "Création…"
                } else {
                    "Créer une branche de récupération"
                },
                true,
            )
            .on_press(|s: &mut State| s.reflog_recover()),
        )
        .child(
            text("La branche courante ne bouge pas : seul un nouveau repère est créé.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
}

pub fn reflog_screen(s: &State) -> El {
    let p = s.palette;
    let context = if s.reflog.loading {
        "Lecture…".to_owned()
    } else {
        counted(s.reflog.entries.len(), "entrée", "entrées")
    };
    let header = screen_head(
        s,
        "rl-head",
        "Reflog",
        context,
        vec![
            action(s, "rl-scope-head", "HEAD", !s.reflog.branch_scope)
                .on_press(|s: &mut State| s.reflog_set_scope(false)),
            action(
                s,
                "rl-scope-branch",
                "Branche courante",
                s.reflog.branch_scope,
            )
            .on_press(|s: &mut State| s.reflog_set_scope(true)),
        ],
    );
    let list: El = if let Some(error) = &s.reflog.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.reflog.entries.is_empty() && !s.reflog.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucune entrée dans ce reflog.".to_owned())
                .key("reflog-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("reflog-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("reflog-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.reflog.version, s.dark))
                    .virtual_rows(
                        s.reflog.entries.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("rlk{i}")),
                        entry_row,
                    ),
            )
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(detail(s)),
    )
}
