use std::borrow::Cow;

use rune::{El, GrowthSpec, Key, ScrollSpec, col, el, row, text, text_input};

use crate::history_state::{HistoryStatus, SearchStatus, format_date};
use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 28.0;
const GLYPH_CELL: f32 = 8.0;

fn commit_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let history = &s.history;
    let (Some(commit), Some(graph)) = (history.commits.get(index), history.rows.get(index)) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = history.selected == Some(index);
    let glyphs = gitpoulpe_core::glyphs(graph, history.width);
    let short: String = commit.oid.chars().take(8).collect();
    let mut line = row()
        .w_full()
        .h(ROW_HEIGHT)
        .px(12.0)
        .gap(10.0)
        .center_y()
        .child(
            el().w(history.width.max(1) as f32 * 2.0 * GLYPH_CELL)
                .child(text(glyphs).font(theme::CODE).color(p.accent)),
        )
        .child(
            text(commit.subject.clone())
                .font(theme::BODY)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if !commit.refs.is_empty() {
        line = line.child(
            text(commit.refs.join(" · "))
                .font(theme::SMALL)
                .color(p.accent)
                .max_w(220.0)
                .truncate(),
        );
    }
    line = line
        .child(
            text(commit.author.clone())
                .font(theme::SMALL)
                .color(p.muted)
                .w(130.0)
                .truncate(),
        )
        .child(
            text(format_date(commit.timestamp))
                .font(theme::SMALL)
                .color(p.muted)
                .w(116.0),
        )
        .child(text(short).font(theme::PATH_SMALL).color(p.muted).w(64.0));
    el().key(format!("hr{index}"))
        .role("option")
        .label(format!("{}, {}", commit.subject, commit.author))
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_commit(index))
        .child(line)
}

fn result_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(commit) = s.search.results.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let oid = commit.oid.clone();
    let short: String = commit.oid.chars().take(8).collect();
    el().key(format!("sr{index}"))
        .role("option")
        .label(format!("{}, {}", commit.subject, commit.author))
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .hover(p.hover)
        .on_press(move |s: &mut State| s.reveal_commit(oid.clone()))
        .child(
            row()
                .w_full()
                .h(ROW_HEIGHT)
                .px(12.0)
                .gap(10.0)
                .center_y()
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
                        .color(p.muted)
                        .w(130.0)
                        .truncate(),
                )
                .child(
                    text(format_date(commit.timestamp))
                        .font(theme::SMALL)
                        .color(p.muted)
                        .w(116.0),
                )
                .child(text(short).font(theme::PATH_SMALL).color(p.muted).w(64.0)),
        )
}

fn results(s: &State) -> El {
    let query = s.search_text.text.trim().to_owned();
    match &s.search.status {
        SearchStatus::Idle | SearchStatus::Loading if s.search.results.is_empty() => {
            message(s, "Recherche…", &format!("« {query} »"))
        }
        SearchStatus::NoResults => message(
            s,
            "Aucun résultat",
            &format!(
                "Aucun commit ne correspond à « {query} » (message, auteur ou début d'identifiant)."
            ),
        ),
        SearchStatus::Failed(error) => message(s, "Recherche impossible", error),
        _ => col()
            .key("search-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("search-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.search.version, s.dark))
                    .virtual_rows(
                        s.search.results.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("sk{i}")),
                        result_row,
                    )
                    .on_visible_rows(|s: &mut State, range| s.search_visible(range)),
            ),
    }
}

fn sequence_card(s: &State) -> Option<El> {
    use gitpoulpe_core::{EmptyPolicy, SequenceKind};
    let p = s.palette;
    let state = s.sequencing.state.as_ref()?;
    let (title, verb) = match state.kind {
        SequenceKind::CherryPick => ("Cherry-pick : commits à reprendre", "Appliquer"),
        SequenceKind::Revert => ("Revert : commits à inverser", "Créer les commits inverses"),
    };
    let mut card = col()
        .key("sequence-card")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text(title.to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text("Ils seront appliqués dans cet ordre, du premier au dernier :".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    for (index, (oid, subject)) in state.queue.iter().enumerate() {
        let short: String = oid.chars().take(8).collect();
        card = card.child(
            row()
                .gap(6.0)
                .center_y()
                .child(
                    text(format!("{}. {short} {subject}", index + 1))
                        .key(format!("seq-item{index}"))
                        .font(theme::PATH_SMALL)
                        .color(p.text)
                        .grow(1.0)
                        .truncate(),
                )
                .child(
                    action(s, &format!("seq-up{index}"), "↑", false)
                        .on_press(move |s: &mut State| s.sequence_move_up(index)),
                )
                .child(
                    action(s, &format!("seq-del{index}"), "×", false)
                        .on_press(move |s: &mut State| s.sequence_remove(index)),
                ),
        );
    }
    if let Some(error) = &state.error {
        card = card.child(
            text(error.clone())
                .key("seq-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let Some(preview) = &state.preview else {
        if state.error.is_none() {
            card = card.child(
                text("Calcul de l'aperçu…".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        return Some(card.child(
            action(s, "seq-close", "Fermer", false).on_press(|s: &mut State| s.sequence_close()),
        ));
    };
    card = card.child(
        text(format!(
            "Sur {} · {} commit(s){}.",
            preview.branch.as_deref().unwrap_or("HEAD détaché"),
            preview.commits.len(),
            if preview.local_changes.is_empty() {
                String::new()
            } else {
                format!(
                    " · {} fichier(s) avec modifications locales",
                    preview.local_changes.len()
                )
            }
        ))
        .key("seq-summary")
        .font(theme::SMALL)
        .color(p.muted)
        .wrap(),
    );
    if preview.merges > 0 {
        let parents = preview.commits.iter().map(|c| c.parents).min().unwrap_or(2);
        let mut choices = row().gap(6.0);
        for parent in 1..=parents as u32 {
            choices = choices.child(
                action(
                    s,
                    &format!("seq-parent{parent}"),
                    &format!("Parent {parent}"),
                    state.mainline == Some(parent),
                )
                .on_press(move |s: &mut State| s.sequence_set_mainline(parent)),
            );
        }
        card = card
            .child(
                text("Commit de fusion : choisissez explicitement le parent à garder comme référence (les fusions ne se mélangent pas avec d'autres commits).".to_owned())
                    .key("seq-merge")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            )
            .child(choices);
    }
    if state.kind == SequenceKind::CherryPick {
        card = card
            .child(
                text("Si un commit devient vide :".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                row()
                    .gap(6.0)
                    .child(
                        action(
                            s,
                            "seq-empty-drop",
                            "l'ignorer",
                            state.empty == EmptyPolicy::Drop,
                        )
                        .on_press(|s: &mut State| s.sequence_set_empty(EmptyPolicy::Drop)),
                    )
                    .child(
                        action(
                            s,
                            "seq-empty-keep",
                            "le garder vide",
                            state.empty == EmptyPolicy::Keep,
                        )
                        .on_press(|s: &mut State| s.sequence_set_empty(EmptyPolicy::Keep)),
                    )
                    .child(
                        action(
                            s,
                            "seq-empty-stop",
                            "m'arrêter",
                            state.empty == EmptyPolicy::Stop,
                        )
                        .on_press(|s: &mut State| s.sequence_set_empty(EmptyPolicy::Stop)),
                    ),
            );
    }
    Some(
        card.child(
            action(
                s,
                "seq-run",
                if state.busy { "En cours…" } else { verb },
                true,
            )
            .on_press(|s: &mut State| s.sequence_run()),
        )
        .child(
            action(s, "seq-close", "Fermer", false).on_press(|s: &mut State| s.sequence_close()),
        ),
    )
}

fn rebase_card(s: &State) -> Option<El> {
    use gitpoulpe_core::RebaseAction;
    let p = s.palette;
    let ui = s.rebasing.state.as_ref()?;
    let mut card = col()
        .key("rebase-card")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text("Rebase interactif".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        );
    if let Some(error) = &ui.error {
        card = card.child(
            text(error.clone())
                .key("rebase-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let Some(info) = &ui.info else {
        if ui.error.is_none() {
            card = card.child(
                text("Lecture de la plage…".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        return Some(card.child(
            action(s, "rebase-close", "Fermer", false).on_press(|s: &mut State| s.rebase_close()),
        ));
    };
    card = card
        .child(
            text(format!(
                "Base : {} · sur {} · {} commit(s) réécrits, du plus ancien au plus récent.",
                info.base.chars().take(8).collect::<String>(),
                info.branch.as_deref().unwrap_or("HEAD détaché"),
                info.commits.len()
            ))
            .key("rebase-summary")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(
            text(
                "Touchez l'action pour la changer (pick, reword, edit, squash, fixup, drop)."
                    .to_owned(),
            )
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    for (index, item) in ui.items.iter().enumerate() {
        let subject = ui.subject_of(&item.oid);
        card = card.child(
            row()
                .gap(6.0)
                .center_y()
                .child(
                    action(
                        s,
                        &format!("rebase-act{index}"),
                        item.action.caption(),
                        item.action != RebaseAction::Pick,
                    )
                    .on_press(move |s: &mut State| s.rebase_cycle(index)),
                )
                .child(
                    el().key(format!("rebase-item{index}"))
                        .role("option")
                        .focusable(true)
                        .grow(1.0)
                        .on_press(move |s: &mut State| s.rebase_select(index))
                        .child(
                            text(format!(
                                "{} {subject}",
                                item.oid.chars().take(8).collect::<String>()
                            ))
                            .font(theme::PATH_SMALL)
                            .color(if ui.selected == Some(index) {
                                p.accent
                            } else {
                                p.text
                            })
                            .truncate(),
                        ),
                )
                .child(
                    action(s, &format!("rebase-up{index}"), "↑", false)
                        .on_press(move |s: &mut State| s.rebase_move_up(index)),
                ),
        );
    }
    if let Some(index) = ui.selected
        && ui
            .items
            .get(index)
            .is_some_and(|i| i.action == RebaseAction::Reword)
    {
        card = card.child(
            crate::widgets::field(
                s,
                "rebase-message",
                "Nouveau message",
                &ui.message,
                "Message",
            )
            .on_edit(|s: &mut State| &mut s.rebasing.state.as_mut().expect("rebase card").message),
        );
    }
    if let Some(problem) = ui.problem() {
        card = card.child(
            text(problem)
                .key("rebase-problem")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if !info.local_changes.is_empty() {
        card = card.child(
            crate::widgets::toggle(
                s,
                "rebase-autostash",
                "Mettre mon travail local de côté pendant le rebase",
                ui.autostash,
            )
            .on_press(|s: &mut State| s.rebase_toggle_autostash()),
        );
    }
    Some(
        card.child(
            text("L'ancien sommet est gardé sous refs/gitpoulpe/recovery. Un conflit ou une étape edit met le rebase en pause : continuez ou abandonnez depuis l'écran de résolution.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
        .child(
            action(
                s,
                "rebase-run",
                if ui.busy { "En cours…" } else { "Lancer le rebase" },
                true,
            )
            .on_press(|s: &mut State| s.rebase_run()),
        )
        .child(
            action(s, "rebase-close", "Fermer", false).on_press(|s: &mut State| s.rebase_close()),
        ),
    )
}

fn reset_card(s: &State, oid: &str) -> Option<El> {
    use gitpoulpe_core::ResetMode;
    let p = s.palette;
    let state = s.resetting.state.as_ref().filter(|r| r.target == oid)?;
    let mut card = col()
        .key("reset-card")
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text("Repositionner la branche courante ici".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        );
    let modes = row()
        .gap(6.0)
        .child(
            action(s, "reset-soft", "soft", state.mode == ResetMode::Soft)
                .on_press(|s: &mut State| s.reset_set_mode(ResetMode::Soft)),
        )
        .child(
            action(s, "reset-mixed", "mixed", state.mode == ResetMode::Mixed)
                .on_press(|s: &mut State| s.reset_set_mode(ResetMode::Mixed)),
        )
        .child(
            action(s, "reset-hard", "hard", state.mode == ResetMode::Hard)
                .on_press(|s: &mut State| s.reset_set_mode(ResetMode::Hard)),
        );
    card = card.child(modes);
    if let Some(done) = &state.done {
        card = card.child(
            text(done.clone())
                .key("reset-done")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &state.error {
        card = card.child(
            text(error.clone())
                .key("reset-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(preview) = &state.preview {
        let branch = preview
            .branch
            .clone()
            .unwrap_or_else(|| "HEAD détaché".to_owned());
        let effect = match state.mode {
            ResetMode::Soft => format!(
                "soft : « {branch} » passe sur ce commit ; l'index et les fichiers ne changent pas, les commits retirés restent en changements indexés."
            ),
            ResetMode::Mixed => format!(
                "mixed : « {branch} » passe sur ce commit et l'index est remis dessus ; les fichiers ne changent pas. {} fichier(s) indexé(s) redeviennent non indexés.",
                preview.staged.len()
            ),
            ResetMode::Hard => format!(
                "hard : « {branch} » passe sur ce commit, l'index et les fichiers suivis sont remis dessus. Les fichiers non suivis ne sont pas touchés."
            ),
        };
        card = card.child(
            text(effect)
                .key("reset-effect")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
        if preview.entering > 0 && preview.leaving.is_empty() {
            card = card.child(
                text(format!(
                    "La branche avance de {} commit(s).",
                    preview.entering
                ))
                .font(theme::SMALL)
                .color(p.muted),
            );
        }
        if !preview.leaving.is_empty() {
            let list = preview
                .leaving
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
            let more = if preview.leaving.len() > 8 || preview.leaving_truncated {
                "\n…"
            } else {
                ""
            };
            card = card.child(
                text(format!(
                    "{} commit(s) ne seront plus sur la branche :\n{list}{more}",
                    preview.leaving.len()
                ))
                .key("reset-leaving")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap(),
            );
        }
        if !preview.would_lose.is_empty() {
            card = card.child(
                text(format!(
                    "Modifications non enregistrées qui seraient effacées : {}.",
                    preview
                        .would_lose
                        .iter()
                        .take(8)
                        .map(|f| f.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .key("reset-lose")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        }
        card = card.child(
            text("Repères : l'ancien sommet est gardé sous refs/gitpoulpe/recovery et dans le reflog ; les modifications suivies non enregistrées sont sauvegardées (git stash create). Limites : les fichiers non suivis ne sont pas touchés et les fichiers ignorés ne sont pas sauvegardés.".to_owned())
                .key("reset-limits")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
        if state.confirm {
            card = card
                .child(
                    text("Confirmez l'effacement des modifications listées ci-dessus.".to_owned())
                        .key("reset-confirm-text")
                        .font(theme::SMALL)
                        .color(p.red)
                        .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "reset-confirm", "Effacer et repositionner", true)
                                .on_press(|s: &mut State| s.reset_run()),
                        )
                        .child(
                            action(s, "reset-confirm-cancel", "Annuler", false)
                                .on_press(|s: &mut State| s.reset_cancel_confirm()),
                        ),
                );
        } else {
            card = card.child(
                action(
                    s,
                    "reset-run",
                    if state.busy {
                        "En cours…"
                    } else {
                        "Repositionner"
                    },
                    true,
                )
                .on_press(|s: &mut State| s.reset_run()),
            );
        }
    } else if state.error.is_none() {
        card = card.child(
            text("Calcul de l'aperçu…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    Some(
        card.child(
            action(s, "reset-close", "Fermer", false).on_press(|s: &mut State| s.reset_close()),
        ),
    )
}

fn action(s: &State, key: &str, caption: &str, primary: bool) -> El {
    crate::widgets::action(s, key, caption, primary)
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("hist-panel")
        .w(340.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(commit) = s.history.selected.and_then(|i| s.history.commits.get(i)) else {
        return panel.child(
            text("Sélectionne un commit pour voir ses détails.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    let parents = if commit.parents.is_empty() {
        "aucun (commit racine)".to_owned()
    } else {
        commit
            .parents
            .iter()
            .map(|oid| oid.chars().take(8).collect::<String>())
            .collect::<Vec<_>>()
            .join(", ")
    };
    panel = panel
        .child(
            text(commit.subject.clone())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!(
                "{} · {} UTC",
                commit.author,
                format_date(commit.timestamp)
            ))
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(
            text(format!("Commit {}", commit.oid))
                .key("hist-detail-oid")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap()
                .selectable(true),
        )
        .child(
            text(format!("Parents : {parents}"))
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap(),
        );
    if !commit.refs.is_empty() {
        panel = panel.child(
            text(format!("Références : {}", commit.refs.join(", ")))
                .font(theme::SMALL)
                .color(p.accent)
                .wrap(),
        );
    }
    let oid = commit.oid.clone();
    let tag_oid = commit.oid.clone();
    let reset_oid = commit.oid.clone();
    let reset_subject = commit.subject.clone();
    panel
        .child(
            el().key("tag-commit")
                .role("button")
                .label("Créer un tag sur ce commit")
                .focusable(true)
                .focus_ring(2.0, 2.0, p.accent)
                .h(32.0)
                .px(12.0)
                .center()
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .hover(p.hover)
                .on_press(move |s: &mut State| s.open_tags(Some(tag_oid.clone())))
                .child(
                    text("Créer un tag ici".to_owned())
                        .font(theme::BODY)
                        .color(p.text),
                ),
        )
        .child(
            el().key("inspect-commit")
                .role("button")
                .label("Inspecter ce commit")
                .focusable(true)
                .focus_ring(2.0, 2.0, p.accent)
                .h(32.0)
                .px(12.0)
                .center()
                .radius(3.0)
                .fill(p.accent)
                .hover(p.hover)
                .on_press(move |s: &mut State| s.open_commit(oid.clone(), 0))
                .child(
                    text("Inspecter ce commit".to_owned())
                        .font(theme::BODY_STRONG)
                        .color(p.on_accent),
                ),
        )
        .child({
            let target = reset_oid.clone();
            action(s, "reset-here", "Repositionner la branche ici…", false)
                .on_press(move |s: &mut State| s.reset_ask(&target))
        })
        .child({
            let (o, subject) = (reset_oid.clone(), reset_subject.clone());
            action(s, "pick-add", "Ajouter au cherry-pick", false).on_press(move |s: &mut State| {
                s.sequence_add(gitpoulpe_core::SequenceKind::CherryPick, &o, &subject)
            })
        })
        .child({
            let (o, subject) = (reset_oid.clone(), reset_subject.clone());
            action(s, "revert-add", "Annuler ce commit (revert)", false).on_press(
                move |s: &mut State| {
                    s.sequence_add(gitpoulpe_core::SequenceKind::Revert, &o, &subject)
                },
            )
        })
        .child({
            let target = reset_oid.clone();
            action(
                s,
                "rebase-here",
                "Rebase interactif depuis ce commit…",
                false,
            )
            .on_press(move |s: &mut State| s.rebase_ask(&target))
        })
        .children(rebase_card(s))
        .children(sequence_card(s))
        .children(reset_card(s, &reset_oid))
}

fn message(s: &State, title: &str, detail: &str) -> El {
    let p = s.palette;
    col().w_full().grow(1.0).center().child(
        col()
            .gap(8.0)
            .max_w(520.0)
            .child(text(title.to_owned()).font(theme::TITLE).color(p.text))
            .child(
                text(detail.to_owned())
                    .font(theme::BODY)
                    .color(p.muted)
                    .wrap(),
            ),
    )
}

pub fn history_screen(s: &State) -> El {
    let p = s.palette;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!(
                "Historique · {} commits chargés",
                s.history.commits.len()
            ))
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            text_input(&s.search_text, theme::BODY)
                .key("history-search")
                .label("Rechercher un commit")
                .single_line()
                .w(320.0)
                .h(34.0)
                .px(10.0)
                .py(8.0)
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .color(p.text)
                .placeholder("Message, auteur ou identifiant".to_owned(), p.muted)
                .focus_ring(2.0, 2.0, p.accent)
                .on_edit(|s: &mut State| &mut s.search_text)
                .on_input_change(|s: &mut State, _| s.search_changed())
                .on_input_key(|s: &mut State, event, tx| {
                    if event.key != Key::Escape {
                        return false;
                    }
                    if s.search_text.text.is_empty() {
                        tx.blur();
                    } else {
                        s.search_text = rune::TextEdit::new("");
                        s.search_changed();
                    }
                    true
                }),
        )
        .child(
            el().key("history-close")
                .role("button")
                .label("Fermer l'historique")
                .focusable(true)
                .focus_ring(2.0, 2.0, p.accent)
                .h(32.0)
                .px(12.0)
                .center()
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .hover(p.hover)
                .on_press(|s: &mut State| s.close_history())
                .child(text("Fermer".to_owned()).font(theme::BODY).color(p.text)),
        );
    let list_body = match &s.history.status {
        HistoryStatus::Idle | HistoryStatus::Loading if s.history.commits.is_empty() => message(
            s,
            "Chargement de l'historique…",
            "Lecture des premiers commits.",
        ),
        HistoryStatus::NoCommits => message(
            s,
            "Aucun commit",
            "Ce dépôt n'a pas encore de commit : crée le premier depuis l'écran des changements.",
        ),
        HistoryStatus::Failed(error) => message(s, "Lecture de l'historique impossible", error),
        _ => {
            let list = col()
                .key("hist-rows")
                .role("listbox")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows_version((s.history.version, s.history.selected, s.dark))
                .virtual_rows(
                    s.history.commits.len(),
                    ROW_HEIGHT,
                    |_: &State, i| Cow::Owned(format!("hk{i}")),
                    commit_row,
                )
                .on_visible_rows(|s: &mut State, range| s.history_visible(range));
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(
                    col()
                        .key("hist-viewport")
                        .grow(1.0)
                        .min_w(0.0)
                        .h_full()
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(list),
                )
                .child(detail(s))
        }
    };
    let body = if s.search_active() {
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(results(s))
            .child(detail(s))
    } else {
        list_body
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
