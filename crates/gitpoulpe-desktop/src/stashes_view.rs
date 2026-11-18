use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::history_state::format_date;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, toggle};

const ROW_HEIGHT: f32 = 48.0;

fn stash_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(stash) = s.stashes.stashes.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.stashes.selected.as_deref() == Some(stash.oid.as_str());
    let oid = stash.oid.clone();
    let title = if stash.message.is_empty() {
        "(sans message)".to_owned()
    } else {
        stash.message.clone()
    };
    let detail = format!(
        "stash@{{{}}} · {} · {} UTC",
        stash.index,
        stash.branch.as_deref().unwrap_or("branche inconnue"),
        format_date(stash.timestamp)
    );
    el().key(format!("st-row{index}"))
        .role("option")
        .label(title.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_stash(&oid))
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
                .child(text(detail).font(theme::SMALL).color(p.muted).truncate()),
        )
}

fn selected_detail(s: &State, mut panel: El) -> El {
    let p = s.palette;
    let Some(stash) = s.stashes.selected_stash() else {
        return panel.child(
            text("Sélectionnez un stash pour l'inspecter, l'appliquer ou le supprimer.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel.child(el().h(1.0).w_full().fill(p.line)).child(
        text(format!("stash@{{{}}} · {}", stash.index, stash.message))
            .key("st-selected")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    match &s.stashes.detail {
        None => {
            panel = panel.child(
                text("Lecture du contenu…".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        Some(detail) => {
            let mut lines: Vec<String> = detail
                .files
                .iter()
                .map(|f| {
                    format!(
                        "{} {}{}",
                        f.status,
                        f.path.display(),
                        if f.staged { "  (indexé)" } else { "" }
                    )
                })
                .collect();
            lines.extend(
                detail
                    .untracked
                    .iter()
                    .map(|p| format!("? {}  (non suivi)", p.display())),
            );
            panel = panel.child(
                text(if lines.is_empty() {
                    "Ce stash ne contient aucun fichier.".to_owned()
                } else {
                    lines.join("\n")
                })
                .key("st-files")
                .font(theme::PATH_SMALL)
                .color(p.text)
                .wrap()
                .selectable(true),
            );
        }
    }
    panel = panel
        .child(
            toggle(
                s,
                "st-restore-index",
                "Restaurer aussi l'état de l'index (--index)",
                s.stashes.restore_index,
            )
            .on_press(|s: &mut State| s.stash_toggle("restore-index")),
        )
        .child(
            action(s, "st-apply", "Appliquer (le stash reste)", true)
                .on_press(|s: &mut State| s.stash_apply(false)),
        )
        .child(
            action(s, "st-pop", "Appliquer et retirer de la liste", false)
                .on_press(|s: &mut State| s.stash_apply(true)),
        );
    if s.stashes.confirm_drop.as_deref() == Some(stash.oid.as_str()) {
        panel.child(
            col()
                .key("st-confirm")
                .w_full()
                .gap(6.0)
                .px(12.0)
                .py(10.0)
                .radius(3.0)
                .border(1.0, p.red)
                .child(
                    text(format!(
                        "Supprimer stash@{{{}}} ? Son contenu reste récupérable tant que Git ne nettoie pas ses objets : {}",
                        stash.index,
                        crate::shell::restore_stash(&stash.message, &stash.oid)
                    ))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "st-drop-confirm", "Supprimer le stash", true)
                                .on_press(|s: &mut State| s.stash_drop()),
                        )
                        .child(
                            action(s, "st-drop-cancel", "Annuler", false)
                                .on_press(|s: &mut State| s.stash_cancel_drop()),
                        ),
                ),
        )
    } else {
        panel.child(
            action(s, "st-drop", "Supprimer ce stash", false)
                .on_press(|s: &mut State| s.stash_ask_drop()),
        )
    }
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let st = &s.stashes;
    let panel = col()
        .key("st-panel")
        .w(420.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text("Mettre le travail de côté".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "stash-message",
                "Message du stash",
                &st.message,
                "Message (facultatif)",
            )
            .on_edit(|s: &mut State| &mut s.stashes.message),
        )
        .child(
            toggle(
                s,
                "st-untracked",
                "Inclure les fichiers non suivis",
                st.include_untracked,
            )
            .on_press(|s: &mut State| s.stash_toggle("untracked")),
        )
        .child(
            toggle(s, "st-keep", "Garder l'index en place", st.keep_index)
                .on_press(|s: &mut State| s.stash_toggle("keep")),
        )
        .child(
            toggle(
                s,
                "st-staged",
                "Seulement les changements indexés",
                st.staged_only,
            )
            .on_press(|s: &mut State| s.stash_toggle("staged")),
        )
        .child(
            action(
                s,
                "st-create",
                if st.busy {
                    "En cours…"
                } else {
                    "Mettre de côté"
                },
                true,
            )
            .on_press(|s: &mut State| s.stash_create()),
        );
    selected_detail(s, panel)
}

pub fn stashes_screen(s: &State) -> El {
    let p = s.palette;
    let title = if s.stashes.loading {
        "Réserve · lecture…".to_owned()
    } else {
        format!("Réserve · {} stash(s)", s.stashes.stashes.len())
    };
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(title)
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(s, "stashes-branches", "Branches", false)
                .on_press(|s: &mut State| s.open_branches()),
        )
        .child(
            action(s, "stashes-close", "Fermer", false).on_press(|s: &mut State| s.close_stashes()),
        );
    let list: El = if let Some(error) = &s.stashes.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.stashes.stashes.is_empty() && !s.stashes.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun stash dans ce dépôt.".to_owned())
                .key("stashes-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("stashes-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("stashes-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.stashes.version, s.dark))
                    .virtual_rows(
                        s.stashes.stashes.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("stk{i}")),
                        stash_row,
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
