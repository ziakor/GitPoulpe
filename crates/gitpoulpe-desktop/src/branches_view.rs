use std::borrow::Cow;

use rune::{El, GrowthSpec, Key, ScrollSpec, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;
use crate::widgets::counted;
use crate::widgets::toggle;

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
    let rule = if selected {
        el().w(2.0).h_full().fill(p.accent)
    } else {
        el().w(2.0).h_full()
    };
    row()
        .key(format!("bk-row{index}"))
        .role("option")
        .label(branch.name.clone())
        .focus_selected(selected)
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .fill(if selected { p.selected } else { p.bg })
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_branch(&name))
        .child(rule)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .h(ROW_HEIGHT)
                .px(14.0)
                .justify_center()
                .gap(2.0)
                .child(title.w_full())
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(p.muted)
                        .w_full()
                        .truncate(),
                ),
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
                "Pour revenir en arrière : {} (aussi noté dans le journal).",
                crate::shell::restore_branch(&confirm.name, &confirm.loss.tip)
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

fn merge_panel(s: &State) -> Option<El> {
    use gitpoulpe_core::MergeKind;
    let p = s.palette;
    let m = s.branches.merge.as_ref()?;
    let mut card = col()
        .key("merge-panel")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line);
    let Some(preview) = &m.preview else {
        let line = match &m.error {
            Some(error) => text(format!("Aperçu impossible : {error}")).color(p.red),
            None => text("Calcul de l'aperçu…".to_owned()).color(p.muted),
        };
        return Some(
            card.child(line.key("merge-wait").font(theme::SMALL).wrap())
                .child(
                    action(s, "merge-cancel", "Fermer", false)
                        .on_press(|s: &mut State| s.merge_cancel()),
                ),
        );
    };
    let current = preview
        .current
        .clone()
        .unwrap_or_else(|| "HEAD détaché".to_owned());
    let kind = match preview.kind {
        MergeKind::UpToDate => format!(
            "« {} » est déjà contenue dans « {current} » : il n'y a rien à fusionner.",
            preview.source
        ),
        MergeKind::FastForward => format!(
            "Avance rapide : « {current} » avance de {} commit(s) jusqu'à « {} », sans commit de fusion.",
            preview.only_in_source, preview.source
        ),
        MergeKind::TrueMerge => format!(
            "Commit de fusion : {} commit(s) de « {} » rejoignent « {current} » ({} commit(s) propres à « {current} »).",
            preview.only_in_source, preview.source, preview.only_in_head
        ),
    };
    card = card
        .child(
            text(format!(
                "Fusionner « {} » dans « {current} »",
                preview.source
            ))
            .key("merge-title")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap(),
        )
        .child(
            text(kind)
                .key("merge-kind")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    match &preview.predicted_conflicts {
        Some(files) if files.is_empty() => {
            card = card.child(
                text("Aucun conflit prévu.".to_owned())
                    .key("merge-no-conflict")
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        Some(files) => {
            card = card.child(
                text(format!(
                    "{} fichier(s) en conflit prévu(s) : {}. La fusion s'arrêtera pour vous laisser les résoudre.",
                    files.len(),
                    files
                        .iter()
                        .take(5)
                        .map(|f| f.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .key("merge-conflicts")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        }
        None => {}
    }
    if !preview.local_changes.is_empty() {
        card = card.child(
            text(format!(
                "{} fichier(s) ont des modifications locales ({}). Git refuse la fusion si elle devait les écraser ; sinon elles sont conservées.",
                preview.local_changes.len(),
                preview
                    .local_changes
                    .iter()
                    .take(5)
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .key("merge-local")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    if let Some(error) = &m.error {
        card = card.child(
            text(format!("Fusion non faite : {error}"))
                .key("merge-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if preview.kind != MergeKind::UpToDate {
        card = card
            .child(
                toggle(s, "merge-no-ff", "Toujours créer un commit de fusion", m.no_ff)
                    .on_press(|s: &mut State| s.merge_toggle_no_ff()),
            )
            .child(
                toggle(
                    s,
                    "merge-autostash",
                    "Mettre mon travail local de côté pendant la fusion (réserve conservée si besoin)",
                    m.autostash,
                )
                .on_press(|s: &mut State| s.merge_toggle_autostash()),
            );
    }
    let mut buttons = row().gap(8.0);
    if preview.kind != MergeKind::UpToDate {
        buttons = buttons.child(
            action(
                s,
                "merge-confirm",
                if m.busy { "Fusion…" } else { "Fusionner" },
                true,
            )
            .on_press(|s: &mut State| s.merge_confirm()),
        );
    }
    buttons = buttons.child(
        action(s, "merge-cancel", "Fermer", false).on_press(|s: &mut State| s.merge_cancel()),
    );
    card = card.child(buttons);
    Some(card)
}

fn pull_panel(s: &State) -> Option<El> {
    use gitpoulpe_core::{PullKind, PullStrategy};
    let p = s.palette;
    let state = s.pulling.state.as_ref()?;
    let mut card = col()
        .key("pull-panel")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text("Intégrer les changements distants".to_owned())
                .key("pull-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        );
    if let Some((phase, percent)) = &state.running {
        return Some(
            card.child(
                text(format!("Intégration · {phase} {percent} %"))
                    .key("pull-running")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                action(s, "pull-stop", "Annuler", false)
                    .on_press(|s: &mut State| s.pull_cancel_running()),
            ),
        );
    }
    if let Some(done) = &state.done {
        card = card.child(
            text(done.clone())
                .key("pull-done")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &state.error {
        card = card.child(
            text(error.clone())
                .key("pull-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(preview) = &state.preview {
        card = card
            .child(
                text(format!(
                    "Branche suivie : {} → {} ({}) · {} commit(s) chez vous, {} chez le distant, d'après la dernière récupération.",
                    preview.upstream,
                    preview.branch,
                    crate::pulling::kind_caption(preview.kind),
                    preview.ahead,
                    preview.behind
                ))
                .key("pull-status")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
            )
            .child(
                text("Stratégie (rien n'est choisi pour vous) :".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                row()
                    .gap(6.0)
                    .child(
                        action(
                            s,
                            "pull-ff",
                            "avance rapide",
                            state.strategy == PullStrategy::FastForwardOnly,
                        )
                        .on_press(|s: &mut State| {
                            s.pull_set_strategy(PullStrategy::FastForwardOnly)
                        }),
                    )
                    .child(
                        action(s, "pull-merge", "fusion", state.strategy == PullStrategy::Merge)
                            .on_press(|s: &mut State| s.pull_set_strategy(PullStrategy::Merge)),
                    )
                    .child(
                        action(s, "pull-rebase", "rebase", state.strategy == PullStrategy::Rebase)
                            .on_press(|s: &mut State| s.pull_set_strategy(PullStrategy::Rebase)),
                    ),
            )
            .child(
                text(format!(
                    "Stratégie choisie : {}. La récupération a lieu d'abord ; si elle échoue ou est annulée, rien n'est intégré et votre travail local reste intact.",
                    state.strategy.caption()
                ))
                .key("pull-strategy")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
            );
        if preview.kind == PullKind::Diverged && state.strategy == PullStrategy::FastForwardOnly {
            card = card.child(
                text("Les branches ont divergé : l'avance rapide sera refusée. Choisissez la fusion ou le rebase.".to_owned())
                    .key("pull-diverged")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
        if !preview.local_changes.is_empty() {
            card = card.child(
                toggle(
                    s,
                    "pull-autostash",
                    "Mettre mon travail local de côté pendant l'intégration",
                    state.autostash,
                )
                .on_press(|s: &mut State| s.pull_toggle_autostash()),
            );
        }
        card = card.child(
            action(s, "pull-run", "Récupérer et intégrer", true)
                .on_press(|s: &mut State| s.pull_run()),
        );
    } else if state.error.is_none() {
        card = card.child(
            text("Lecture de la branche suivie…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    Some(
        card.child(
            action(s, "pull-close", "Fermer", false).on_press(|s: &mut State| s.pull_close()),
        ),
    )
}

fn publish_panel(s: &State) -> Option<El> {
    use gitpoulpe_core::PushKind;
    let p = s.palette;
    let state = s.publishing.state.as_ref()?;
    let mut card = col()
        .key("publish-panel")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text(format!("Publier « {} »", state.branch))
                .key("publish-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        );
    if state.remotes.len() > 1 {
        let mut choices = row().gap(6.0);
        for (index, remote) in state.remotes.iter().enumerate() {
            let chosen = state.remote.as_deref() == Some(remote.as_str());
            let name = remote.clone();
            choices = choices.child(
                action(s, &format!("publish-remote{index}"), remote, chosen)
                    .on_press(move |s: &mut State| s.publish_choose_remote(&name)),
            );
        }
        card = card.child(choices);
    }
    if let Some((phase, percent)) = &state.running {
        return Some(
            card.child(
                text(format!("Envoi · {phase} {percent} %"))
                    .key("publish-running")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                action(s, "publish-stop", "Annuler l'envoi", false)
                    .on_press(|s: &mut State| s.publish_cancel_running()),
            ),
        );
    }
    if let Some(done) = &state.done {
        card = card.child(
            text(done.clone())
                .key("publish-done")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &state.error {
        card = card.child(
            text(error.clone())
                .key("publish-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(preview) = &state.preview {
        let status = match preview.kind {
            PushKind::NewBranch => format!(
                "Nouvelle branche sur {} : {} sera créée avec votre historique.",
                preview.remote, preview.destination
            ),
            PushKind::FastForward => format!(
                "{} commit(s) à envoyer vers {} ({}), sans rien écraser.",
                preview.ahead, preview.remote, preview.destination
            ),
            PushKind::UpToDate => format!(
                "Déjà à jour sur {} ({}).",
                preview.remote, preview.destination
            ),
            PushKind::Behind => format!(
                "Le distant a {} commit(s) de plus que votre branche : récupérez et intégrez-les ; rien à envoyer.",
                preview.behind
            ),
            PushKind::Diverged => format!(
                "Votre branche et {} ont divergé ({} commit(s) chez vous, {} chez le distant). Un envoi simple serait refusé.",
                preview.remote, preview.ahead, preview.behind
            ),
        };
        card = card
            .child(
                text(format!(
                    "Destination : {} → {}  (d'après la dernière récupération)",
                    preview.remote, preview.destination
                ))
                .key("publish-destination")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
            )
            .child(
                text(status)
                    .key("publish-status")
                    .font(theme::SMALL)
                    .color(p.text)
                    .wrap(),
            );
        if preview.upstream.is_none() && preview.kind != PushKind::UpToDate {
            card = card.child(
                toggle(
                    s,
                    "publish-upstream",
                    "Suivre cette branche distante (upstream)",
                    state.set_upstream,
                )
                .on_press(|s: &mut State| s.publish_toggle_upstream()),
            );
        }
        if matches!(preview.kind, PushKind::NewBranch | PushKind::FastForward) {
            card = card.child(
                action(s, "publish-confirm", "Publier", true)
                    .on_press(|s: &mut State| s.publish_run(false)),
            );
        }
        if preview.kind == PushKind::Diverged && preview.known_remote_oid.is_some() {
            if state.confirm_force {
                let expected = preview
                    .known_remote_oid
                    .as_deref()
                    .map(|o| o.chars().take(10).collect::<String>())
                    .unwrap_or_default();
                let lost = preview
                    .would_discard
                    .iter()
                    .take(8)
                    .map(|c| {
                        format!(
                            "{} {}",
                            c.oid.chars().take(8).collect::<String>(),
                            c.subject
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                card = card
                    .child(
                        text(format!(
                            "Publication protégée : le distant sera remplacé seulement s'il est toujours à {expected} ; sinon l'envoi est refusé. Ces commits distants ne seront plus atteignables depuis la branche :\n{lost}"
                        ))
                        .key("publish-force-warning")
                        .font(theme::SMALL)
                        .color(p.red)
                        .wrap(),
                    )
                    .child(
                        row()
                            .gap(8.0)
                            .child(
                                action(s, "publish-force-confirm", "Remplacer le distant", true)
                                    .on_press(|s: &mut State| s.publish_run(true)),
                            )
                            .child(
                                action(s, "publish-force-cancel", "Annuler", false)
                                    .on_press(|s: &mut State| s.publish_cancel_force()),
                            ),
                    );
            } else {
                card = card.child(
                    action(s, "publish-force", "Publier l'historique réécrit…", false)
                        .on_press(|s: &mut State| s.publish_ask_force()),
                );
            }
        }
    } else if state.error.is_none() {
        card = card.child(
            text("Lecture de l'état distant connu…".to_owned())
                .key("publish-wait")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    card = card.child(
        action(s, "publish-close", "Fermer", false).on_press(|s: &mut State| s.publish_close()),
    );
    Some(card)
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("branch-panel")
        .w(380.0)
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
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
                "Sélectionnez une branche pour la changer, la renommer ou la supprimer.".to_owned(),
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
    if branch.current {
        panel = panel.child(
            action(
                s,
                "branch-pull",
                "Intégrer les changements distants…",
                false,
            )
            .on_press(|s: &mut State| s.pull_ask()),
        );
        if let Some(card) = pull_panel(s) {
            panel = panel.child(card);
        }
    }
    panel = panel.child(
        action(s, "branch-publish", "Publier…", false).on_press(|s: &mut State| s.publish_ask()),
    );
    if let Some(card) = publish_panel(s) {
        panel = panel.child(card);
    }
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
        let into = s.branches.branches.iter().find(|b| b.current).map_or_else(
            || "la branche courante".to_owned(),
            |b| format!("« {} »", b.name),
        );
        panel = panel.child(
            action(s, "branch-merge", &format!("Fusionner dans {into}…"), false)
                .on_press(|s: &mut State| s.merge_ask()),
        );
        if let Some(merge) = merge_panel(s) {
            panel = panel.child(merge);
        }
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
    let context = if s.branches.loading {
        "Lecture…".to_owned()
    } else {
        let count = counted(
            s.branches.branches.len(),
            "branche locale",
            "branches locales",
        );
        match s.branches.branches.iter().find(|b| b.current) {
            Some(current) => format!("{count} · courante : {}", current.name),
            None => count,
        }
    };
    let header = crate::widgets::screen_head(
        s,
        "branches-head",
        "Branches",
        context,
        vec![
            action(s, "branches-pulls", "Nouvelle pull request…", false)
                .on_press(|s: &mut State| s.open_pulls()),
        ],
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
