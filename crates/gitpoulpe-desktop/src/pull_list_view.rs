use std::borrow::Cow;

use gitpoulpe_hosting::{
    CheckState, MergeMethod, PullFilter, PullState, PullSummary, ReviewDecision,
};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::pull_list::{CommentKind, numbered_lines, review_state_label, short};
use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, toggle};

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
    panel = panel.child(
        action(s, "prl-git", "Lire toute la PR depuis Git", false)
            .on_press(|s: &mut State| s.open_pull_from_git()),
    );
    if detail.files_truncated || detail.files.len() >= 3000 {
        panel = panel.child(
            text("L'API GitHub plafonne la liste à 3000 fichiers : utilisez « Lire toute la PR depuis Git » pour tous les fichiers.".to_owned())
                .key("prl-cap")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if !detail.body.trim().is_empty() {
        panel = panel.child(
            text(detail.body.clone())
                .key("prl-body")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    panel = panel
        .child(checks_panel(s))
        .child(merge_panel(s))
        .child(compose(s))
        .child(review_panel(s));
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
                for (n, numbered) in numbered_lines(patch).into_iter().enumerate() {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    let line = numbered.text;
                    let picked = list.pick.as_ref().is_some_and(|k| {
                        k.path == file.change.path
                            && numbered.anchor.is_some_and(|(l, side)| {
                                side == k.side && l >= k.start.unwrap_or(k.line) && l <= k.line
                            })
                    });
                    let mut line_el = row()
                        .key(format!("prl-f{i}l{n}"))
                        .w_full()
                        .px(6.0)
                        .child(text(line.clone()).font(theme::CODE).color(p.text));
                    if line.starts_with('+') {
                        line_el = line_el.fill(p.added);
                    } else if line.starts_with('-') {
                        line_el = line_el.fill(p.removed);
                    }
                    if picked {
                        line_el = line_el.fill(p.selected);
                    }
                    if let Some((number, side)) = numbered.anchor {
                        let path = file.change.path.clone();
                        line_el = line_el.on_press(move |s: &mut State| {
                            s.pull_pick_line(path.clone(), number, side)
                        });
                    }
                    panel = panel.child(line_el);
                }
                panel = panel.child(threads(s, &file.change.path, i));
            }
        }
    }
    panel = panel.child(
        text(format!(
            "{} commentaire(s) généraux",
            s.pulls.list.comments.issue.len()
        ))
        .key("prc-general-title")
        .font(theme::BODY_STRONG)
        .color(p.text),
    );
    for (i, comment) in s.pulls.list.comments.issue.iter().enumerate() {
        panel = panel.child(
            text(format!("{} : {}", comment.author, comment.body))
                .key(format!("prc-general{i}"))
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
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

fn compose(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let target = match (&list.reply_to, &list.pick) {
        (Some(id), _) => format!("Réponse au commentaire #{id}"),
        (None, Some(k)) => format!(
            "Ligne {}{} ({}) de {}",
            k.start.map(|s| format!("{s}–")).unwrap_or_default(),
            k.line,
            if k.side == gitpoulpe_hosting::Side::Left {
                "ancien côté"
            } else {
                "nouveau côté"
            },
            k.path
        ),
        (None, None) => "Commentaire général (cliquez une ligne de diff pour l'ancrer)".to_owned(),
    };
    let mut block = col()
        .gap(6.0)
        .child(
            text("Commenter".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            text(target)
                .key("prc-target")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
        .child(
            toggle(
                s,
                "prc-range",
                "Sélectionner une plage de lignes",
                list.range_mode,
            )
            .on_press(|s: &mut State| s.pull_toggle_range()),
        )
        .child(
            field(
                s,
                "prc-body",
                "Commentaire",
                &list.compose,
                "Votre commentaire",
            )
            .on_edit(|s: &mut State| &mut s.pulls.list.compose),
        );
    let mut buttons = row().gap(8.0);
    if list.reply_to.is_some() {
        buttons = buttons.child(
            action(s, "prc-send-reply", "Répondre", true)
                .on_press(|s: &mut State| s.pull_send_comment(CommentKind::Reply)),
        );
    } else {
        if list.pick.is_some() {
            buttons = buttons.child(
                action(s, "prc-send-line", "Commenter la ligne", true)
                    .on_press(|s: &mut State| s.pull_send_comment(CommentKind::Line)),
            );
        }
        if list.pick.is_some() {
            buttons = buttons.child(
                action(s, "prc-add-draft", "Ajouter à la revue", false)
                    .on_press(|s: &mut State| s.pull_add_to_draft()),
            );
        }
        buttons = buttons.child(
            action(
                s,
                "prc-send-general",
                "Commentaire général",
                list.pick.is_none(),
            )
            .on_press(|s: &mut State| s.pull_send_comment(CommentKind::General)),
        );
    }
    block = block.child(buttons);
    if list.sending {
        block = block.child(
            text("Envoi…".to_owned())
                .key("prc-sending")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(notice) = &list.send_notice {
        block = block.child(
            text(notice.clone())
                .key("prc-notice")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &list.send_error {
        block = block.child(
            text(error.clone())
                .key("prc-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if list.unverified.is_some() {
        block = block
            .child(
                text("Le résultat de l'envoi est inconnu : vérifiez avant de réessayer, pour ne pas publier deux fois.".to_owned())
                    .key("prc-unknown")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            )
            .child(
                action(s, "prc-verify", "Vérifier si le commentaire est parti", false)
                    .on_press(|s: &mut State| s.pull_verify_comment()),
            );
    }
    block
}

fn method_caption(method: MergeMethod) -> (&'static str, &'static str, &'static str) {
    match method {
        MergeMethod::Merge => (
            "mrg-merge",
            "Fusionner (commit de fusion)",
            "Crée un commit de fusion ; tous les commits de la branche sont conservés dans l'historique.",
        ),
        MergeMethod::Squash => (
            "mrg-squash",
            "Fusionner en un seul commit",
            "Regroupe tous les commits de la branche en un seul commit sur la branche cible.",
        ),
        MergeMethod::Rebase => (
            "mrg-rebase",
            "Rebaser et fusionner",
            "Rejoue les commits sur la branche cible sans commit de fusion ; leurs identifiants changent.",
        ),
    }
}

fn merge_panel(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut block = col().gap(4.0).child(
        text("Fusion".to_owned())
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    if list.merge_loading {
        block = block.child(
            text("Lecture des règles de fusion…".to_owned())
                .key("mrg-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(notice) = &list.merge_notice {
        block = block.child(
            text(notice.clone())
                .key("mrg-notice")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &list.merge_error {
        block = block.child(
            text(error.clone())
                .key("mrg-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if list.merge_unverified {
        block = block
            .child(
                text("Le résultat de la fusion est inconnu : vérifiez l'état distant avant de réessayer.".to_owned())
                    .key("mrg-unknown")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            )
            .child(
                action(s, "mrg-verify", "Vérifier l'état distant", false)
                    .on_press(|s: &mut State| s.pull_merge_verify()),
            );
    }
    let Some(ready) = &list.merge else {
        return block;
    };
    if ready.protection_unreadable {
        block = block.child(
            text("Les règles de protection de la branche ne sont pas lisibles avec ce compte : elles seront appliquées par GitHub à la fusion.".to_owned())
                .key("mrg-protection")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    if ready.blockers.is_empty() {
        block = block.child(
            text("Aucun point bloquant connu.".to_owned())
                .key("mrg-clear")
                .font(theme::SMALL)
                .color(p.muted),
        );
    } else {
        for (i, blocker) in ready.blockers.iter().enumerate() {
            block = block.child(
                text(format!("• {blocker}"))
                    .key(format!("mrg-blocker{i}"))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
    }
    if !ready.can_attempt() {
        return block;
    }
    if let Some(method) = list.merge_confirm {
        let (_, caption, effect) = method_caption(method);
        block = block
            .child(
                text(format!(
                    "Confirmer : {caption} sur la révision {} ? {effect}",
                    short(&ready.head_sha)
                ))
                .key("mrg-confirm-text")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
            )
            .child(
                field(
                    s,
                    "mrg-message",
                    "Message du commit (facultatif)",
                    &list.merge_message,
                    "Message",
                )
                .on_edit(|s: &mut State| &mut s.pulls.list.merge_message),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(
                        action(
                            s,
                            "mrg-confirm",
                            if list.merging {
                                "Fusion…"
                            } else {
                                "Confirmer la fusion"
                            },
                            true,
                        )
                        .on_press(|s: &mut State| s.pull_merge_confirmed()),
                    )
                    .child(
                        action(s, "mrg-cancel", "Annuler", false)
                            .on_press(|s: &mut State| s.pull_merge_cancel()),
                    ),
            );
    } else {
        for method in &ready.methods {
            let (key, caption, effect) = method_caption(*method);
            let method = *method;
            block = block
                .child(
                    action(s, key, caption, false)
                        .on_press(move |s: &mut State| s.pull_merge_choose(method)),
                )
                .child(
                    text(effect.to_owned())
                        .font(theme::SMALL)
                        .color(p.muted)
                        .wrap(),
                );
        }
    }
    block
}

fn check_label(state: CheckState) -> &'static str {
    match state {
        CheckState::Pending => "en attente",
        CheckState::Success => "réussie",
        CheckState::Failure => "échouée",
        CheckState::Cancelled => "annulée",
        CheckState::Skipped => "ignorée",
    }
}

fn checks_panel(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut block = col().gap(4.0).child(
        text("Vérifications".to_owned())
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    if let Some(note) = &list.checks_note {
        block = block.child(
            text(note.clone())
                .key("chk-note")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if list.checks_loading {
        block = block.child(
            text("Chargement des vérifications…".to_owned())
                .key("chk-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(error) = &list.checks_error {
        block = block.child(
            text(format!("Vérifications indisponibles : {error}"))
                .key("chk-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(checks) = &list.checks {
        if checks.items.is_empty() {
            block = block.child(
                text(format!(
                    "Aucune vérification pour la révision {}.",
                    short(&checks.sha)
                ))
                .key("chk-none")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
            );
        } else {
            block = block.child(
                text(format!(
                    "Révision {} : {} en attente, {} réussie(s), {} échouée(s), {} annulée(s), {} ignorée(s)",
                    short(&checks.sha),
                    checks.count(CheckState::Pending),
                    checks.count(CheckState::Success),
                    checks.count(CheckState::Failure),
                    checks.count(CheckState::Cancelled),
                    checks.count(CheckState::Skipped)
                ))
                .key("chk-summary")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
            );
            for (i, check) in checks.items.iter().enumerate() {
                block = block.child(
                    text(format!(
                        "{} : {}{}{}",
                        check.name,
                        check_label(check.state),
                        if check.legacy { " (statut)" } else { "" },
                        if check.url.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", check.url)
                        }
                    ))
                    .key(format!("chk-item{i}"))
                    .font(theme::PATH_SMALL)
                    .color(if check.state == CheckState::Failure {
                        p.red
                    } else {
                        p.muted
                    })
                    .truncate(),
                );
            }
            if checks.truncated {
                block = block.child(
                    text("Liste tronquée.".to_owned())
                        .font(theme::SMALL)
                        .color(p.muted),
                );
            }
        }
    }
    block
}

fn decision_button(s: &State, key: &str, caption: &str, decision: ReviewDecision) -> El {
    action(s, key, caption, s.pulls.list.decision == decision)
        .on_press(move |st: &mut State| st.pull_set_decision(decision))
}

fn review_panel(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let draft = s.current_draft();
    let count = draft.map(|d| d.entries.len()).unwrap_or(0);
    let mut block = col().gap(6.0).child(
        text(format!(
            "Revue en tant que {}@{} · {count} commentaire(s) dans le brouillon",
            s.pull_login(),
            s.pulls
                .candidates
                .get(s.pulls.choice)
                .map(|c| c.connection.host_caption())
                .unwrap_or_default()
        ))
        .key("prv-title")
        .font(theme::BODY_STRONG)
        .color(p.text)
        .wrap(),
    );
    if let Some(draft) = draft {
        if draft.stale {
            block = block
                .child(
                    text(format!(
                        "La pull request a été mise à jour depuis la préparation du brouillon (révision {}) : relisez les éléments signalés.",
                        short(&draft.head_sha)
                    ))
                    .key("prv-stale")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    action(s, "prv-adopt", "Adopter la révision actuelle", false)
                        .on_press(|s: &mut State| s.pull_adopt_head()),
                );
        }
        for (i, entry) in draft.entries.iter().enumerate() {
            block = block.child(
                text(format!(
                    "{}:{}{} ({}) — {}{}",
                    entry.line.path,
                    entry
                        .line
                        .start_line
                        .map(|l| format!("{l}–"))
                        .unwrap_or_default(),
                    entry.line.line,
                    if entry.line.side == gitpoulpe_hosting::Side::Left {
                        "ancien"
                    } else {
                        "nouveau"
                    },
                    entry.line.body,
                    if entry.to_reread {
                        " — À RELIRE"
                    } else {
                        ""
                    }
                ))
                .key(format!("prv-entry{i}"))
                .font(theme::SMALL)
                .color(if entry.to_reread { p.red } else { p.muted })
                .wrap(),
            );
            let mut buttons = row().gap(8.0);
            if entry.to_reread {
                buttons = buttons.child(
                    action(s, &format!("prv-read{i}"), "Marquer relu", false)
                        .on_press(move |s: &mut State| s.pull_mark_read(i)),
                );
            }
            buttons = buttons.child(
                action(s, &format!("prv-del{i}"), "Retirer", false)
                    .on_press(move |s: &mut State| s.pull_remove_draft_line(i)),
            );
            block = block.child(buttons);
        }
    }
    block = block
        .child(
            field(
                s,
                "prv-body",
                "Message de la revue",
                &list.review_body,
                "Message de la revue",
            )
            .on_edit(|s: &mut State| &mut s.pulls.list.review_body),
        )
        .child(
            row()
                .gap(8.0)
                .child(decision_button(
                    s,
                    "prv-dec-comment",
                    "Commenter",
                    ReviewDecision::Comment,
                ))
                .child(decision_button(
                    s,
                    "prv-dec-approve",
                    "Approuver",
                    ReviewDecision::Approve,
                ))
                .child(decision_button(
                    s,
                    "prv-dec-changes",
                    "Demander des changements",
                    ReviewDecision::RequestChanges,
                )),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    action(
                        s,
                        "prv-submit",
                        if list.review_sending {
                            "Envoi…"
                        } else {
                            "Soumettre la revue"
                        },
                        true,
                    )
                    .on_press(|s: &mut State| s.pull_submit_review()),
                )
                .child(
                    action(s, "prv-discard", "Abandonner le brouillon", false)
                        .on_press(|s: &mut State| s.pull_discard_draft()),
                ),
        );
    if let Some(notice) = &list.review_notice {
        block = block.child(
            text(notice.clone())
                .key("prv-notice")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &list.review_error {
        block = block.child(
            text(error.clone())
                .key("prv-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if list.review_unverified {
        block = block
            .child(
                text("Le résultat de l'envoi est inconnu : vérifiez avant de réessayer, pour ne pas publier deux fois.".to_owned())
                    .key("prv-unknown")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            )
            .child(
                action(s, "prv-verify", "Vérifier si la revue est partie", false)
                    .on_press(|s: &mut State| s.pull_verify_review()),
            );
    }
    block = block.child(
        text(format!("{} revue(s) publiée(s)", list.reviews.len()))
            .key("prv-published-title")
            .font(theme::SMALL)
            .color(p.muted),
    );
    for (i, review) in list.reviews.iter().enumerate() {
        block = block.child(
            text(format!(
                "{} : {} sur {}",
                review.author,
                review_state_label(&review.state),
                short(&review.commit_sha)
            ))
            .key(format!("prv-published{i}"))
            .font(theme::PATH_SMALL)
            .color(p.muted)
            .truncate(),
        );
    }
    block
}

fn threads(s: &State, path: &str, file_index: usize) -> El {
    let p = s.palette;
    let comments = &s.pulls.list.comments;
    let mut column = col().gap(4.0).px(6.0);
    for root in comments
        .review
        .iter()
        .filter(|c| c.path == path && c.in_reply_to.is_none())
    {
        let id = root.id;
        column = column.child(
            text(format!(
                "{} : {} (ligne {}{})",
                root.author,
                root.body,
                root.line
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "?".to_owned()),
                if root.outdated { ", périmé" } else { "" }
            ))
            .key(format!("prc-f{file_index}c{id}"))
            .font(theme::SMALL)
            .color(p.text)
            .wrap(),
        );
        for reply in comments
            .review
            .iter()
            .filter(|c| c.in_reply_to == Some(root.id))
        {
            column = column.child(
                text(format!("    ↳ {} : {}", reply.author, reply.body))
                    .key(format!("prc-f{file_index}r{}", reply.id))
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
        }
        column = column.child(
            action(s, &format!("prc-reply{id}"), "Répondre", false)
                .on_press(move |s: &mut State| s.pull_reply_to(id)),
        );
    }
    column
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
