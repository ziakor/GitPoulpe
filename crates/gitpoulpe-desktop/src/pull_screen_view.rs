use rune::{El, ScrollSpec, col, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, toggle};

pub fn pulls_screen(s: &State) -> El {
    let p = s.palette;
    let pulls = &s.pulls;
    let branch = pulls.branch.clone().unwrap_or_else(|| "?".to_owned());
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!("Pull request · {branch}"))
                .key("pr-title-line")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(s, "pr-refresh", "Actualiser", false)
                .on_press(|s: &mut State| s.pulls_refresh()),
        )
        .child(action(s, "pr-close", "Fermer", false).on_press(|s: &mut State| s.close_pulls()));
    let mut body = col()
        .key("pr-panel")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .py(14.0)
        .gap(10.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(notice) = &pulls.notice {
        body = body.child(
            text(notice.clone())
                .key("pr-notice")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &pulls.error {
        body = body.child(
            text(error.clone())
                .key("pr-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if pulls.loading {
        body = body.child(
            text("Préparation de la pull request…".to_owned())
                .key("pr-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(candidate) = pulls.candidates.get(pulls.choice) {
        body = body.child(
            text(format!(
                "Compte : {}@{} · dépôt distant {} ({}/{})",
                candidate.connection.login,
                candidate.connection.host_caption(),
                candidate.remote,
                candidate.reference.owner,
                candidate.reference.repo
            ))
            .key("pr-account")
            .font(theme::SMALL)
            .color(p.text)
            .wrap(),
        );
        if pulls.candidates.len() > 1 {
            body = body.child(
                action(s, "pr-account-next", "Changer de compte", false)
                    .on_press(|s: &mut State| s.pulls_next_account()),
            );
        }
    }
    let Some(target) = &pulls.target else {
        return col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(header)
            .child(body);
    };
    body = body.child(
        text(format!(
            "{}:{} → {}:{}",
            target.head_repo.full_name(),
            target.branch,
            target.target_repo.full_name(),
            target.base
        ))
        .key("pr-direction")
        .font(theme::PATH_SMALL)
        .color(p.text)
        .wrap(),
    );
    if target.head_repo.parent.is_some() {
        body = body.child(
            toggle(
                s,
                "pr-parent",
                "Cibler le dépôt d'origine du fork",
                pulls.to_parent,
            )
            .on_press(|s: &mut State| s.pulls_toggle_parent()),
        );
    }
    if !target.published {
        body = body.child(
            text(format!(
                "La branche {} n'est pas publiée sur {} : publiez-la d'abord (écran Branches), puis actualisez.",
                target.branch,
                target.head_repo.full_name()
            ))
            .key("pr-unpublished")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    if let Some(existing) = &target.existing {
        body = body.child(
            text(format!(
                "Une pull request est déjà ouverte : #{} {} ({}).",
                existing.number, existing.title, existing.url
            ))
            .key("pr-existing")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    if let Some(comparison) = &target.comparison {
        body = body.child(
            text(format!(
                "{} commit(s) d'avance, {} de retard · {} fichier(s) modifié(s){}",
                comparison.ahead,
                comparison.behind,
                comparison.files.len(),
                if comparison.files_truncated {
                    " (liste tronquée)"
                } else {
                    ""
                }
            ))
            .key("pr-summary")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap(),
        );
        for (i, file) in comparison.files.iter().enumerate().take(200) {
            body = body.child(
                text(format!(
                    "{} {} (+{} −{})",
                    file.status, file.path, file.additions, file.deletions
                ))
                .key(format!("pr-file{i}"))
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .truncate(),
            );
        }
    }
    if target.published && target.existing.is_none() {
        body = body
            .child(
                field(s, "pr-base", "Branche cible", &pulls.base, "main")
                    .on_edit(|s: &mut State| &mut s.pulls.base),
            )
            .child(
                field(
                    s,
                    "pr-title",
                    "Titre",
                    &pulls.title,
                    "Titre de la pull request",
                )
                .on_edit(|s: &mut State| &mut s.pulls.title),
            )
            .child(
                field(
                    s,
                    "pr-body",
                    "Description",
                    &pulls.body,
                    "Description (facultative)",
                )
                .on_edit(|s: &mut State| &mut s.pulls.body),
            )
            .child(
                toggle(s, "pr-draft", "Créer comme brouillon", pulls.draft)
                    .on_press(|s: &mut State| s.pulls_toggle_draft()),
            )
            .child(
                action(
                    s,
                    "pr-create",
                    if pulls.busy {
                        "Création…"
                    } else {
                        "Créer la pull request"
                    },
                    true,
                )
                .on_press(|s: &mut State| s.create_pull()),
            );
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
