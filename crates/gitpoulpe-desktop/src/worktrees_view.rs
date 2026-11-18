use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, toggle};

const ROW_HEIGHT: f32 = 52.0;

fn state_line(w: &gitpoulpe_core::Worktree) -> String {
    if w.prunable.is_some() {
        return "dossier disparu".to_owned();
    }
    if !w.dirty() {
        return "propre".to_owned();
    }
    format!(
        "{} indexé(s) · {} modifié(s) · {} non suivi(s)",
        w.staged, w.unstaged, w.untracked
    )
}

fn worktree_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(w) = s.worktrees.list.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.worktrees.selected.as_ref() == Some(&w.path);
    let path = w.path.clone();
    let title = format!(
        "{}{}{}",
        w.branch.as_deref().unwrap_or("HEAD détaché"),
        if w.main { " · principal" } else { "" },
        if w.current { " · ouvert" } else { "" },
    );
    let detail = format!(
        "{} · {}{}",
        w.path.display(),
        state_line(w),
        if w.locked.is_some() {
            " · verrouillé"
        } else {
            ""
        }
    );
    el().key(format!("wt-row{index}"))
        .role("option")
        .label(title.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_worktree(path.clone()))
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
    let Some(w) = s.worktrees.selected_worktree() else {
        return panel.child(
            text("Sélectionnez un worktree : chacun a son propre état de travail.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel
        .child(el().h(1.0).w_full().fill(p.line))
        .child(
            text(w.path.display().to_string())
                .key("wt-selected")
                .font(theme::PATH_SMALL)
                .color(p.text)
                .wrap()
                .selectable(true),
        )
        .child(
            text(format!(
                "{} · {}",
                w.branch.as_deref().unwrap_or("HEAD détaché"),
                state_line(w)
            ))
            .key("wt-state")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    if let Some(reason) = &w.locked {
        panel = panel.child(
            text(format!(
                "Verrouillé{}",
                if reason.is_empty() {
                    String::new()
                } else {
                    format!(" : {reason}")
                }
            ))
            .key("wt-locked")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    if let Some(reason) = &w.prunable {
        panel = panel.child(
            text(format!(
                "Dossier absent ({reason}) : « Nettoyer » retire l'entrée orpheline."
            ))
            .key("wt-prunable")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    if !w.current && w.prunable.is_none() {
        panel = panel.child(
            action(s, "wt-open", "Ouvrir ce worktree", true)
                .on_press(|s: &mut State| s.worktree_open()),
        );
    }
    if !w.main {
        if w.locked.is_some() {
            panel = panel.child(
                action(s, "wt-unlock", "Déverrouiller", false)
                    .on_press(|s: &mut State| s.worktree_unlock()),
            );
        } else {
            panel = panel
                .child(
                    field(
                        s,
                        "wt-reason",
                        "Raison du verrouillage",
                        &s.worktrees.lock_reason,
                        "Raison (facultatif)",
                    )
                    .on_edit(|s: &mut State| &mut s.worktrees.lock_reason),
                )
                .child(
                    action(s, "wt-lock", "Verrouiller", false)
                        .on_press(|s: &mut State| s.worktree_lock()),
                );
        }
        if !w.current {
            panel = panel.child(
                action(s, "wt-remove", "Supprimer ce worktree…", false)
                    .on_press(|s: &mut State| s.worktree_ask_remove()),
            );
        }
    }
    if let Some(removal) = &s.worktrees.removal {
        let dirty = !removal.changed.is_empty() || !removal.untracked.is_empty();
        let mut card = col()
            .key("wt-confirm")
            .w_full()
            .gap(6.0)
            .px(12.0)
            .py(10.0)
            .radius(3.0)
            .border(1.0, p.red);
        if let Some(reason) = &removal.locked {
            card = card.child(
                text(format!(
                    "Verrouillé{} : déverrouillez-le d'abord.",
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!(" ({reason})")
                    }
                ))
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        } else if dirty {
            card = card
                .child(
                    text(format!(
                        "Ce worktree contient du travail non enregistré. Fichiers modifiés : {}. Fichiers non suivis : {}.",
                        removal.changed.iter().take(6).map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "),
                        removal.untracked.iter().take(6).map(|f| f.display().to_string()).collect::<Vec<_>>().join(", ")
                    ))
                    .key("wt-dirty")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    text("Supprimer sauvegarde les modifications suivies (refs/gitpoulpe/recovery) mais pas les fichiers non suivis, qui seront perdus. La branche est conservée.".to_owned())
                        .font(theme::SMALL)
                        .color(p.muted)
                        .wrap(),
                )
                .child(
                    action(s, "wt-remove-force", "Supprimer et perdre ce travail", true)
                        .on_press(|s: &mut State| s.worktree_remove(true)),
                );
        } else {
            card = card
                .child(
                    text("Ce worktree est propre : seul son dossier est supprimé, la branche est conservée.".to_owned())
                        .font(theme::SMALL)
                        .color(p.text)
                        .wrap(),
                )
                .child(
                    action(s, "wt-remove-confirm", "Supprimer le worktree", true)
                        .on_press(|s: &mut State| s.worktree_remove(false)),
                );
        }
        panel = panel.child(
            card.child(
                action(s, "wt-remove-cancel", "Annuler", false)
                    .on_press(|s: &mut State| s.worktree_cancel_remove()),
            ),
        );
    }
    panel
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let panel = col()
        .key("wt-panel")
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
            text("Nouveau worktree".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "wt-path",
                "Chemin du worktree",
                &s.worktrees.new_path,
                "/chemin/absolu/nouveau-dossier",
            )
            .on_edit(|s: &mut State| &mut s.worktrees.new_path),
        )
        .child(
            field(
                s,
                "wt-branch",
                "Branche",
                &s.worktrees.new_branch,
                "nom-de-branche",
            )
            .on_edit(|s: &mut State| &mut s.worktrees.new_branch),
        )
        .child(
            toggle(
                s,
                "wt-create",
                "Créer cette branche (sinon, branche existante)",
                s.worktrees.create_branch,
            )
            .on_press(|s: &mut State| s.worktree_toggle_create()),
        )
        .child(
            action(
                s,
                "wt-add",
                if s.worktrees.busy {
                    "En cours…"
                } else {
                    "Créer le worktree"
                },
                true,
            )
            .on_press(|s: &mut State| s.worktree_add()),
        );
    selected_detail(s, panel)
}

pub fn worktrees_screen(s: &State) -> El {
    let p = s.palette;
    let title = if s.worktrees.loading {
        "Worktrees · lecture…".to_owned()
    } else {
        format!("Worktrees · {}", s.worktrees.list.len())
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
            action(s, "wt-prune", "Nettoyer les orphelins", false)
                .on_press(|s: &mut State| s.worktree_prune()),
        )
        .child(
            action(s, "wt-branches", "Branches", false).on_press(|s: &mut State| s.open_branches()),
        )
        .child(
            action(s, "wt-close", "Fermer", false).on_press(|s: &mut State| s.close_worktrees()),
        );
    let list: El = if let Some(error) = &s.worktrees.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else {
        col()
            .key("worktrees-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("worktrees-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.worktrees.version, s.dark))
                    .virtual_rows(
                        s.worktrees.list.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("wtk{i}")),
                        worktree_row,
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
