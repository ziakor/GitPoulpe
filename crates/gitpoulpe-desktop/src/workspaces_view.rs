use rune::{El, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, screen_head};
use crate::workspaces::{RepoHealth, health};

fn status(health: RepoHealth) -> (&'static str, bool) {
    match health {
        RepoHealth::Ok => ("accessible", false),
        RepoHealth::Missing => ("introuvable", true),
        RepoHealth::NotRepository => ("n'est plus un dépôt Git", true),
    }
}

pub fn workspaces_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.workspaces;
    let header = screen_head(
        s,
        "ws-head",
        "Espaces de travail",
        String::new(),
        Vec::new(),
    );
    let mut list = col()
        .key("ws-list")
        .w(280.0)
        .h_full()
        .gap(6.0)
        .px(16.0)
        .py(10.0);
    for (index, workspace) in screen.book.items.iter().enumerate() {
        let id = workspace.id;
        let active = screen.book.active == Some(id);
        list = list.child(
            el().key(format!("ws-row{index}"))
                .role("option")
                .label(workspace.name.clone())
                .w_full()
                .h(40.0)
                .px(10.0)
                .radius(3.0)
                .fill(if screen.selected == Some(id) {
                    p.selected
                } else {
                    p.bg
                })
                .hover(p.hover)
                .on_press(move |s: &mut State| s.workspace_select(id))
                .child(
                    row().w_full().h(40.0).center_y().child(
                        text(format!(
                            "{}{} · {} dépôt(s)",
                            workspace.name,
                            if active { " (actif)" } else { "" },
                            workspace.repos.len()
                        ))
                        .font(theme::BODY)
                        .color(p.text)
                        .truncate(),
                    ),
                ),
        );
    }
    list = list
        .child(
            field(
                s,
                "ws-new-name",
                "Nom du nouvel espace",
                &screen.new_name,
                "Nom de l'espace",
            )
            .on_edit(|s: &mut State| &mut s.workspaces.new_name),
        )
        .child(
            action(s, "ws-create", "Créer l'espace", true)
                .on_press(|s: &mut State| s.workspace_create()),
        );
    let mut detail = col()
        .key("ws-detail")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(error) = &screen.error {
        detail = detail.child(
            text(error.clone())
                .key("ws-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &screen.notice {
        detail = detail.child(
            text(notice.clone())
                .key("ws-notice")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    let selected = screen.selected.and_then(|id| screen.book.get(id));
    match selected {
        None => {
            detail = detail.child(
                text("Créez un espace de travail pour regrouper vos dépôts.".to_owned())
                    .key("ws-hint")
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        Some(workspace) => {
            detail = detail
                .child(
                    field(s, "ws-name", "Nom de l'espace", &screen.rename, "Nom")
                        .on_edit(|s: &mut State| &mut s.workspaces.rename),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "ws-rename", "Renommer", false)
                                .on_press(|s: &mut State| s.workspace_rename()),
                        )
                        .child(
                            action(s, "ws-restore", "Restaurer l'espace", true)
                                .on_press(|s: &mut State| s.workspace_restore()),
                        )
                        .child(
                            action(s, "ws-delete", "Supprimer l'espace", false)
                                .on_press(|s: &mut State| s.workspace_delete()),
                        ),
                );
            if screen.confirm_delete {
                detail = detail
                    .child(
                        text("Supprimer cet espace ? Seule la liste est supprimée : aucun fichier de dépôt n'est touché.".to_owned())
                            .key("ws-confirm")
                            .font(theme::SMALL)
                            .color(p.red)
                            .wrap(),
                    )
                    .child(
                        row()
                            .gap(8.0)
                            .child(action(s, "ws-confirm-delete", "Oui, supprimer l'espace", false).on_press(|s: &mut State| s.workspace_delete()))
                            .child(action(s, "ws-cancel-delete", "Annuler", true).on_press(|s: &mut State| s.workspace_cancel_delete())),
                    );
            }
            if workspace.repos.is_empty() {
                detail = detail.child(
                    text("Aucun dépôt dans cet espace.".to_owned())
                        .key("ws-empty")
                        .font(theme::SMALL)
                        .color(p.muted),
                );
            }
            for (index, repo) in workspace.repos.iter().enumerate() {
                let (word, broken) = status(health(&repo.path));
                let (open_path, remove_path, repair_path) =
                    (repo.path.clone(), repo.path.clone(), repo.path.clone());
                let mut line = row()
                    .key(format!("ws-repo{index}"))
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                text(repo.path.clone())
                                    .font(theme::PATH)
                                    .color(p.text)
                                    .truncate(),
                            )
                            .child(
                                text(format!(
                                    "{word} · vue : {}{}",
                                    if repo.view == "staged" {
                                        "indexés"
                                    } else {
                                        "non indexés"
                                    },
                                    repo.file
                                        .as_ref()
                                        .map(|f| format!(" · dernier fichier : {f}"))
                                        .unwrap_or_default()
                                ))
                                .font(theme::SMALL)
                                .color(if broken { p.red } else { p.muted })
                                .truncate(),
                            ),
                    )
                    .child(
                        action(s, &format!("ws-open{index}"), "Ouvrir", false).on_press(
                            move |s: &mut State| s.workspace_open_repo(open_path.clone()),
                        ),
                    );
                if broken {
                    line = line.child(
                        action(s, &format!("ws-repair{index}"), "Réparer…", true).on_press_tx(
                            move |s: &mut State, tx| {
                                s.workspace_repair_folder(repair_path.clone(), tx)
                            },
                        ),
                    );
                }
                line = line.child(
                    action(s, &format!("ws-remove{index}"), "Retirer", false)
                        .on_press(move |s: &mut State| s.workspace_remove_repo(&remove_path)),
                );
                detail = detail.child(line);
            }
            detail = detail.child(
                row()
                    .gap(8.0)
                    .child(
                        action(s, "ws-add-current", "Ajouter le dépôt courant", false)
                            .on_press(|s: &mut State| s.workspace_add_current()),
                    )
                    .child(
                        action(s, "ws-add-folder", "Ajouter un dossier…", false)
                            .on_press_tx(|s: &mut State, tx| s.workspace_add_folder(tx)),
                    ),
            );
        }
    }
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(detail),
    )
}
