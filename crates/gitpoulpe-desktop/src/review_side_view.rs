use gitpoulpe_hosting::{Check, CheckState, MergeMethod, ReviewDecision, Side};
use rune::{El, col, row, text};

use crate::pull_list::short;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, area, choice, field, quiet};

fn heading(s: &State, title: &str, aside: Option<String>) -> El {
    let p = s.palette;
    let mut line = row().w_full().center_y().gap(8.0).child(
        text(title.to_owned())
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0),
    );
    if let Some(aside) = aside {
        line = line.child(text(aside).font(theme::PATH_SMALL).color(p.muted));
    }
    line
}

fn note(s: &State, key: &str, words: String, alarming: bool) -> El {
    let p = s.palette;
    text(words)
        .key(key.to_owned())
        .font(theme::SMALL)
        .color(if alarming { p.red } else { p.muted })
        .w_full()
        .wrap()
}

fn sentence(words: &str) -> String {
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>() + ".",
        None => String::new(),
    }
}

fn check_word(state: CheckState) -> &'static str {
    match state {
        CheckState::Pending => "en attente",
        CheckState::Success => "réussie",
        CheckState::Failure => "échouée",
        CheckState::Cancelled => "annulée",
        CheckState::Skipped => "ignorée",
    }
}

fn needs_attention(check: &Check) -> bool {
    check.state == CheckState::Failure
}

fn check_row(s: &State, index: usize, check: &Check) -> El {
    let p = s.palette;
    let ink = match check.state {
        CheckState::Failure => p.red,
        CheckState::Success => p.green,
        CheckState::Pending => p.text,
        _ => p.muted,
    };
    let mut line = row()
        .key(format!("chk-item{index}"))
        .w_full()
        .gap(10.0)
        .py(3.0)
        .px(4.0)
        .radius(3.0)
        .center_y()
        .child(
            text(check_word(check.state).to_owned())
                .font(theme::TINY)
                .color(ink)
                .w(62.0),
        )
        .child(
            text(format!(
                "{}{}",
                check.name,
                if check.legacy { " (statut)" } else { "" }
            ))
            .font(theme::SMALL)
            .color(p.text)
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        );
    if !check.url.is_empty() {
        let url = check.url.clone();
        line = line
            .role("link")
            .label(format!("{} : ouvrir dans le navigateur", check.name))
            .focusable(true)
            .hover(p.hover)
            .on_press_tx(move |s: &mut State, tx| s.open_link(tx, &url));
    }
    line
}

fn checks_section(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let revision = list
        .checks
        .as_ref()
        .map(|c| format!("révision {}", short(&c.sha)));
    let mut block =
        col()
            .key("chk-section")
            .w_full()
            .gap(6.0)
            .child(heading(s, "Vérifications", revision));
    if let Some(words) = &list.checks_note {
        block = block.child(note(s, "chk-note", words.clone(), true));
    }
    if list.checks_loading {
        block = block.child(note(
            s,
            "chk-loading",
            "Chargement des vérifications…".to_owned(),
            false,
        ));
    }
    if let Some(error) = &list.checks_error {
        block = block.child(note(
            s,
            "chk-error",
            format!("Vérifications indisponibles : {error}"),
            true,
        ));
    }
    let Some(checks) = &list.checks else {
        return block;
    };
    if checks.items.is_empty() {
        return block.child(note(
            s,
            "chk-none",
            "Aucune vérification pour cette révision.".to_owned(),
            false,
        ));
    }
    let mut counts = row().key("chk-summary").w_full().gap(0.0).flex_wrap();
    let mut first = true;
    for (state, word, ink) in [
        (CheckState::Failure, "échouée", p.red),
        (CheckState::Pending, "en attente", p.text),
        (CheckState::Cancelled, "annulée", p.muted),
        (CheckState::Success, "réussie", p.green),
        (CheckState::Skipped, "ignorée", p.muted),
    ] {
        let n = checks.count(state);
        if n == 0 {
            continue;
        }
        if !first {
            counts = counts.child(text(" · ".to_owned()).font(theme::SMALL).color(p.muted));
        }
        first = false;
        let word = if n > 1 && state != CheckState::Pending {
            format!("{word}s")
        } else {
            word.to_owned()
        };
        counts = counts.child(text(format!("{n} {word}")).font(theme::SMALL).color(ink));
    }
    block = block.child(counts);
    let attention = checks.items.iter().filter(|c| needs_attention(c)).count();
    let others = checks.items.len() - attention;
    for (i, check) in checks.items.iter().enumerate() {
        if list.checks_expanded || needs_attention(check) {
            block = block.child(check_row(s, i, check));
        }
    }
    if attention == 0 && !list.checks_expanded {
        let pending = checks.count(CheckState::Pending) > 0;
        block = block.child(note(
            s,
            "chk-clear",
            if pending {
                "Aucun échec pour l'instant.".to_owned()
            } else {
                "Aucun échec.".to_owned()
            },
            false,
        ));
    }
    if others > 0 {
        let caption = if list.checks_expanded {
            "Ne montrer que les échecs".to_owned()
        } else if attention == 0 {
            format!("Afficher les {others} vérifications")
        } else {
            format!("Afficher les {others} autres")
        };
        block = block
            .child(quiet(s, "chk-more", &caption).on_press(|s: &mut State| s.toggle_all_checks()));
    }
    if checks.truncated {
        block = block.child(note(
            s,
            "chk-truncated",
            "Liste tronquée.".to_owned(),
            false,
        ));
    }
    block
}

fn method_caption(method: MergeMethod) -> (&'static str, &'static str, &'static str) {
    match method {
        MergeMethod::Merge => (
            "mrg-merge",
            "Commit de fusion",
            "Tous les commits de la branche restent dans l'historique.",
        ),
        MergeMethod::Squash => (
            "mrg-squash",
            "Un seul commit",
            "Les commits de la branche sont regroupés en un commit sur la cible.",
        ),
        MergeMethod::Rebase => (
            "mrg-rebase",
            "Rebaser",
            "Les commits sont rejoués sans commit de fusion ; leurs identifiants changent.",
        ),
    }
}

fn merge_section(s: &State) -> El {
    let list = &s.pulls.list;
    let mut block = col()
        .key("mrg-section")
        .w_full()
        .gap(6.0)
        .child(heading(s, "Fusion", None));
    if list.merge_loading {
        block = block.child(note(
            s,
            "mrg-loading",
            "Lecture des règles de fusion…".to_owned(),
            false,
        ));
    }
    if let Some(notice) = &list.merge_notice {
        block = block.child(
            text(notice.clone())
                .key("mrg-notice")
                .font(theme::SMALL)
                .color(s.palette.text)
                .w_full()
                .wrap(),
        );
    }
    if let Some(error) = &list.merge_error {
        block = block.child(note(s, "mrg-error", error.clone(), true));
    }
    if list.merge_unverified {
        block = block
            .child(note(
                s,
                "mrg-unknown",
                "Le résultat de la fusion est inconnu : vérifiez l'état distant avant de réessayer."
                    .to_owned(),
                true,
            ))
            .child(
                action(s, "mrg-verify", "Vérifier l'état distant", false)
                    .on_press(|s: &mut State| s.pull_merge_verify()),
            );
    }
    let Some(ready) = &list.merge else {
        return block;
    };
    if ready.protection_unreadable {
        block = block.child(note(
            s,
            "mrg-protection",
            "Les règles de protection de la branche ne sont pas lisibles avec ce compte : le serveur les appliquera à la fusion.".to_owned(),
            false,
        ));
    }
    if ready.blockers.is_empty() {
        block = block.child(note(
            s,
            "mrg-clear",
            "Aucun point bloquant connu.".to_owned(),
            false,
        ));
    } else {
        for (i, blocker) in ready.blockers.iter().enumerate() {
            block = block.child(note(s, &format!("mrg-blocker{i}"), sentence(blocker), true));
        }
    }
    if !ready.can_attempt() {
        return block;
    }
    for method in &ready.methods {
        let (key, caption, effect) = method_caption(*method);
        let method = *method;
        block = block.child(
            choice(
                s,
                key,
                caption,
                Some(effect),
                list.merge_confirm == Some(method),
            )
            .on_press(move |s: &mut State| s.pull_merge_choose(method)),
        );
    }
    if let Some(method) = list.merge_confirm {
        let (_, caption, _) = method_caption(method);
        block = block
            .child(
                text(format!(
                    "{caption} sur la révision {} : confirmez pour fusionner.",
                    short(&ready.head_sha)
                ))
                .key("mrg-confirm-text")
                .font(theme::SMALL)
                .color(s.palette.text)
                .w_full()
                .wrap(),
            )
            .child(
                field(
                    s,
                    "mrg-message",
                    "Message du commit (facultatif)",
                    &list.merge_message,
                    "Message du commit (facultatif)",
                )
                .on_edit(|s: &mut State| &mut s.pulls.list.merge_message),
            )
            .child(
                row()
                    .gap(8.0)
                    .center_y()
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
                        quiet(s, "mrg-cancel", "Annuler")
                            .on_press(|s: &mut State| s.pull_merge_cancel()),
                    ),
            );
    }
    block
}

fn decision(s: &State, key: &str, caption: &str, hint: &str, which: ReviewDecision) -> El {
    choice(s, key, caption, Some(hint), s.pulls.list.decision == which)
        .on_press(move |s: &mut State| s.pull_set_decision(which))
}

fn review_section(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let draft = s.current_draft();
    let count = draft.map(|d| d.entries.len()).unwrap_or(0);
    let host = s
        .pulls
        .candidates
        .get(s.pulls.choice)
        .map(|c| c.connection.host_caption())
        .unwrap_or_default();
    let mut block = col()
        .key("prv-section")
        .w_full()
        .gap(6.0)
        .child(heading(s, "Votre revue", None))
        .child(note(
            s,
            "prv-title",
            format!("En tant que {} sur {host}", s.pull_login()),
            false,
        ));
    if let Some(draft) = draft
        && draft.stale
    {
        block = block
            .child(note(
                s,
                "prv-stale",
                format!(
                    "La pull request a changé depuis le début du brouillon (révision {}) : relisez les commentaires signalés.",
                    short(&draft.head_sha)
                ),
                true,
            ))
            .child(
                action(s, "prv-adopt", "Adopter la révision actuelle", false)
                    .on_press(|s: &mut State| s.pull_adopt_head()),
            );
    }
    if count > 0 {
        block = block.child(note(
            s,
            "prv-count",
            if count == 1 {
                "1 commentaire de ligne en attente".to_owned()
            } else {
                format!("{count} commentaires de ligne en attente")
            },
            false,
        ));
    } else {
        block = block.child(note(
            s,
            "prv-empty",
            "Aucun commentaire de ligne en attente : cliquez une ligne dans Fichiers pour en ajouter.".to_owned(),
            false,
        ));
    }
    if let Some(draft) = draft {
        for (i, entry) in draft.entries.iter().enumerate() {
            let place = format!(
                "{}:{}{} ({})",
                entry.line.path,
                entry
                    .line
                    .start_line
                    .map(|l| format!("{l}–"))
                    .unwrap_or_default(),
                entry.line.line,
                if entry.line.side == Side::Left {
                    "ancien"
                } else {
                    "nouveau"
                }
            );
            let mut actions = row().gap(6.0);
            if entry.to_reread {
                actions = actions.child(
                    quiet(s, &format!("prv-read{i}"), "Marquer relu")
                        .on_press(move |s: &mut State| s.pull_mark_read(i)),
                );
            }
            actions = actions.child(
                quiet(s, &format!("prv-del{i}"), "Retirer")
                    .on_press(move |s: &mut State| s.pull_remove_draft_line(i)),
            );
            block = block.child(
                col()
                    .w_full()
                    .gap(2.0)
                    .py(6.0)
                    .border_sides([1.0, 0.0, 0.0, 0.0], p.line)
                    .child(
                        text(place)
                            .font(theme::PATH_SMALL)
                            .color(if entry.to_reread { p.red } else { p.muted })
                            .w_full()
                            .truncate(),
                    )
                    .child(
                        text(format!(
                            "{}{}",
                            entry.line.body,
                            if entry.to_reread {
                                " — à relire"
                            } else {
                                ""
                            }
                        ))
                        .key(format!("prv-entry{i}"))
                        .font(theme::SMALL)
                        .color(p.text)
                        .w_full()
                        .wrap(),
                    )
                    .child(actions),
            );
        }
    }
    block = block
        .child(
            area(
                s,
                "prv-body",
                "Message de la revue",
                &list.review_body,
                "Message de la revue",
            )
            .on_edit(|s: &mut State| &mut s.pulls.list.review_body),
        )
        .child(decision(
            s,
            "prv-dec-comment",
            "Commenter",
            "Des remarques, sans décision.",
            ReviewDecision::Comment,
        ))
        .child(decision(
            s,
            "prv-dec-approve",
            "Approuver",
            "Les changements peuvent être fusionnés.",
            ReviewDecision::Approve,
        ));
    if s.pull_is_gitlab() {
        block = block.child(note(
            s,
            "prv-dec-changes-note",
            "Demander des changements n'est pas possible par l'API GitLab.".to_owned(),
            false,
        ));
    } else {
        block = block.child(decision(
            s,
            "prv-dec-changes",
            "Demander des changements",
            "À corriger avant toute fusion.",
            ReviewDecision::RequestChanges,
        ));
    }
    block = block.child(
        row()
            .flex_wrap()
            .gap(8.0)
            .center_y()
            .pt(4.0)
            .child(
                action(
                    s,
                    "prv-submit",
                    if list.review_sending {
                        "Envoi…"
                    } else {
                        "Envoyer la revue"
                    },
                    true,
                )
                .on_press(|s: &mut State| s.pull_submit_review()),
            )
            .child(
                quiet(s, "prv-discard", "Abandonner le brouillon")
                    .on_press(|s: &mut State| s.pull_discard_draft()),
            ),
    );
    if let Some(notice) = &list.review_notice {
        block = block.child(
            text(notice.clone())
                .key("prv-notice")
                .font(theme::SMALL)
                .color(p.text)
                .w_full()
                .wrap(),
        );
    }
    if let Some(error) = &list.review_error {
        block = block.child(note(s, "prv-error", error.clone(), true));
    }
    if list.review_unverified {
        block = block
            .child(note(
                s,
                "prv-unknown",
                "Le résultat de l'envoi est inconnu : vérifiez avant de réessayer, pour ne pas publier deux fois.".to_owned(),
                true,
            ))
            .child(
                action(s, "prv-verify", "Vérifier si la revue est partie", false)
                    .on_press(|s: &mut State| s.pull_verify_review()),
            );
    }
    block
}

pub fn side_sections(s: &State) -> El {
    col()
        .w_full()
        .gap(24.0)
        .pb(24.0)
        .child(checks_section(s))
        .child(merge_section(s))
        .child(review_section(s))
}
