use std::borrow::Cow;

use rune::{El, GrowthSpec, Key, ScrollSpec, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 44.0;

fn short(oid: &str) -> String {
    oid.chars().take(8).collect()
}

fn action(s: &State, key: &str, caption: &str, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key.to_owned())
        .role("button")
        .label(caption.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .min_h(32.0)
        .px(12.0)
        .py(6.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .child(text(caption.to_owned()).font(theme::BODY).color(ink).wrap())
}

fn branch_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(branch) = s.branches.branches.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.branches.selected.as_deref() == Some(branch.name.as_str());
    let name = branch.name.clone();
    let mut title = row().gap(8.0).center_y().child(
        text(branch.name.clone())
            .font(theme::BODY_STRONG)
            .color(if branch.current { p.accent } else { p.text })
            .truncate(),
    );
    if branch.current {
        title = title.child(
            text("courante".to_owned())
                .font(theme::SMALL)
                .color(p.accent),
        );
    }
    if let Some(path) = &branch.other_worktree {
        title = title.child(
            text(format!("worktree {}", path.display()))
                .font(theme::SMALL)
                .color(p.muted)
                .truncate(),
        );
    }
    let mut detail = format!("{} · {}", short(&branch.oid), branch.subject);
    if let Some(upstream) = &branch.upstream {
        detail = format!("{detail} · suit {upstream}");
    }
    el().key(format!("bk-row{index}"))
        .role("option")
        .label(branch.name.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_branch(&name))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(title)
                .child(text(detail).font(theme::SMALL).color(p.muted).truncate()),
        )
}

fn field(s: &State, key: &str, label: &str, edit: &rune::TextEdit, placeholder: &str) -> El {
    let p = s.palette;
    text_input(edit, theme::BODY)
        .key(key.to_owned())
        .label(label.to_owned())
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(placeholder.to_owned(), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
        .on_input_key(|_: &mut State, event, tx| {
            if event.key == Key::Escape {
                tx.blur();
                return true;
            }
            false
        })
}

fn confirmation(s: &State) -> Option<El> {
    let p = s.palette;
    let confirm = s.branches.confirm.as_ref()?;
    let mut panel = col()
        .key("branch-confirm")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.red)
        .child(
            text(format!(
                "« {} » n'est pas fusionnée : la supprimer fait perdre {}{} commit{} qu'aucune autre branche n'atteint.",
                confirm.name,
                if confirm.loss.truncated { "au moins " } else { "" },
                confirm.loss.commits.len(),
                if confirm.loss.commits.len() > 1 { "s" } else { "" },
            ))
            .font(theme::BODY_STRONG)
            .color(p.red)
            .wrap(),
        );
    for commit in confirm.loss.commits.iter().take(8) {
        panel = panel.child(
            text(format!("{}  {}", short(&commit.oid), commit.subject))
                .font(theme::PATH_SMALL)
                .color(p.text)
                .truncate(),
        );
    }
    if confirm.loss.commits.len() > 8 {
        panel = panel.child(
            text(format!("… et {} autres", confirm.loss.commits.len() - 8))
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    panel = panel
        .child(
            text(format!(
                "Pour revenir en arrière : git branch {} {} (aussi noté dans le journal).",
                confirm.name, confirm.loss.tip
            ))
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    action(s, "branch-delete-force", "Supprimer quand même", true)
                        .on_press(|s: &mut State| s.branch_delete_confirmed()),
                )
                .child(
                    action(s, "branch-delete-cancel", "Annuler", false)
                        .on_press(|s: &mut State| s.branch_cancel_delete()),
                ),
        );
    Some(panel)
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .w(380.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            text("Nouvelle branche à partir de HEAD".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "branch-new",
                "Nom de la nouvelle branche",
                &s.branches.new_name,
                "nom-de-branche",
            )
            .on_edit(|s: &mut State| &mut s.branches.new_name),
        )
        .child(
            action(s, "branch-create", "Créer la branche", false)
                .on_press(|s: &mut State| s.branch_create()),
        );
    let Some(branch) = s.branches.selected_branch() else {
        return panel.child(
            text(
                "Sélectionne une branche pour la changer, la renommer ou la supprimer.".to_owned(),
            )
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    };
    panel = panel.child(el().h(1.0).w_full().fill(p.line)).child(
        text(branch.name.clone())
            .key("branch-selected")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    if let Some(path) = &branch.other_worktree {
        panel = panel.child(
            text(format!(
                "Cette branche est déjà extraite dans le worktree {} : elle ne peut ni y être sélectionnée ici ni être supprimée tant qu'il l'utilise.",
                path.display()
            ))
            .key("branch-worktree-note")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    if !branch.current && branch.other_worktree.is_none() {
        panel = panel
            .child(
                action(s, "branch-switch", "Passer sur cette branche", true)
                    .on_press(|s: &mut State| s.branch_switch()),
            )
            .child(
                text("Les modifications locales qui seraient écrasées bloquent le changement : Git les protège.".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
    }
    panel = panel
        .child(
            field(
                s,
                "branch-rename-to",
                "Nouveau nom",
                &s.branches.rename_to,
                "nouveau-nom",
            )
            .on_edit(|s: &mut State| &mut s.branches.rename_to),
        )
        .child(
            action(s, "branch-rename", "Renommer", false)
                .on_press(|s: &mut State| s.branch_rename()),
        );
    if !branch.current && branch.other_worktree.is_none() {
        panel = panel.child(
            action(s, "branch-delete", "Supprimer", false)
                .on_press(|s: &mut State| s.branch_delete()),
        );
    }
    if let Some(confirm) = confirmation(s) {
        panel = panel.child(confirm);
    }
    panel
}

pub fn branches_screen(s: &State) -> El {
    let p = s.palette;
    let title = if s.branches.loading {
        "Branches · lecture…".to_owned()
    } else {
        format!("Branches · {}", s.branches.branches.len())
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
            action(s, "branches-close", "Fermer", false)
                .on_press(|s: &mut State| s.close_branches()),
        );
    let list: El = if let Some(error) = &s.branches.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else {
        col()
            .key("branches-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("branches-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.branches.version, s.dark))
                    .virtual_rows(
                        s.branches.branches.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("bk{i}")),
                        branch_row,
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
