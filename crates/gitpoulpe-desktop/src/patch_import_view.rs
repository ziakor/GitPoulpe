use gitpoulpe_git::ApplyMode;
use rune::{El, ScrollSpec, col, row, text};

use crate::patch_import::{PatchTab, SourceKind, mode_caption};
use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, field};

fn mode_button(s: &State, key: &str, mode: ApplyMode) -> El {
    action(s, key, mode_caption(mode), s.patches.mode == mode)
        .on_press(move |s: &mut State| s.patch_set_mode(mode))
}

fn source_button(s: &State, key: &str, label: &str, kind: SourceKind) -> El {
    action(s, key, label, s.patches.source == kind)
        .on_press(move |s: &mut State| s.patch_set_source(kind))
}

fn export_body(s: &State, mut body: El) -> El {
    let p = s.palette;
    let screen = &s.patches;
    body = body
        .child(
            text("Source".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            row()
                .gap(8.0)
                .child(source_button(
                    s,
                    "pt-src-tree",
                    "Changements locaux (fichiers suivis)",
                    SourceKind::WorkingTree,
                ))
                .child(source_button(s, "pt-src-index", "Index", SourceKind::Index))
                .child(source_button(
                    s,
                    "pt-src-commits",
                    "Commits",
                    SourceKind::Commits,
                )),
        );
    if screen.source == SourceKind::Commits {
        body = body.child(
            field(
                s,
                "pt-revision",
                "Commit ou plage (HEAD, abc123, main..HEAD)",
                &screen.revision,
                "HEAD~2..HEAD",
            )
            .on_edit(|s: &mut State| &mut s.patches.revision),
        );
    } else {
        body = body.child(
            text("Les fichiers non suivis ne sont pas inclus : indexez-les d'abord pour les exporter.".to_owned())
                .key("pt-untracked")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    body.child(
        field(
            s,
            "pt-dest",
            "Fichier de destination (chemin absolu)",
            &screen.destination,
            "/home/moi/changements.patch",
        )
        .on_edit(|s: &mut State| &mut s.patches.destination),
    )
    .child(
        crate::widgets::toggle(
            s,
            "pt-overwrite",
            "Remplacer le fichier s'il existe",
            screen.overwrite,
        )
        .on_press(|s: &mut State| s.patch_toggle_overwrite()),
    )
    .child(action(s, "pt-export", "Exporter", true).on_press(|s: &mut State| s.patch_export()))
}

pub fn patch_import_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.patches;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(match (screen.tab, screen.name.is_empty()) {
                (PatchTab::Export, _) => "Exporter en patch".to_owned(),
                (PatchTab::Import, true) => "Importer un patch".to_owned(),
                (PatchTab::Import, false) => format!("Importer un patch · {}", screen.name),
            })
            .key("pt-title")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            action(
                s,
                "pt-tab-import",
                "Importer",
                screen.tab == PatchTab::Import,
            )
            .on_press(|s: &mut State| s.patch_set_tab(PatchTab::Import)),
        )
        .child(
            action(
                s,
                "pt-tab-export",
                "Exporter",
                screen.tab == PatchTab::Export,
            )
            .on_press(|s: &mut State| s.patch_set_tab(PatchTab::Export)),
        )
        .child(if screen.tab == PatchTab::Import {
            action(s, "pt-pick", "Choisir un fichier…", true)
                .on_press_tx(|s: &mut State, tx| s.patch_pick(tx))
        } else {
            row()
        })
        .child(
            action(s, "pt-close", "Fermer", false).on_press(|s: &mut State| s.close_patch_import()),
        );
    let mut body = col()
        .key("pt-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .py(8.0)
        .gap(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if screen.busy {
        body = body.child(
            text(if screen.tab == PatchTab::Export {
                "Export en cours…".to_owned()
            } else {
                "Lecture du patch…".to_owned()
            })
            .key("pt-busy")
            .font(theme::SMALL)
            .color(p.muted),
        );
    }
    if let Some(error) = &screen.error {
        body = body.child(
            text(error.clone())
                .key("pt-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &screen.notice {
        body = body.child(
            text(notice.clone())
                .key("pt-notice")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    if screen.tab == PatchTab::Export {
        return col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(header)
            .child(export_body(s, body));
    }
    match &screen.preview {
        None if screen.error.is_none() && !screen.busy => {
            body = body.child(
                text("Choisissez un fichier patch (git diff, git format-patch) : il sera inspecté avant toute application.".to_owned())
                    .key("pt-hint")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
        }
        None => {}
        Some(preview) => {
            body = body.child(
                text(counted(
                    preview.files.len(),
                    "fichier ciblé",
                    "fichiers ciblés",
                ))
                .font(theme::BODY_STRONG)
                .color(p.text),
            );
            if preview.has_message {
                body = body.child(
                    text("Ce patch vient d'un courriel (git format-patch) : son message de commit est ignoré, seuls les fichiers sont appliqués.".to_owned())
                        .key("pt-message")
                        .font(theme::SMALL)
                        .color(p.muted)
                        .wrap(),
                );
            }
            for (index, file) in preview.files.iter().enumerate() {
                body = body.child(
                    text(format!(
                        "{} · {}{}",
                        file.path,
                        if file.binary {
                            "binaire".to_owned()
                        } else {
                            format!("+{} −{}", file.additions, file.deletions)
                        },
                        file.summary
                            .as_ref()
                            .map(|s| format!(" · {s}"))
                            .unwrap_or_default()
                    ))
                    .key(format!("pt-file{index}"))
                    .font(theme::PATH)
                    .color(p.text)
                    .truncate(),
                );
            }
            body = body
                .child(
                    text("Où appliquer".to_owned())
                        .font(theme::BODY_STRONG)
                        .color(p.text),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(mode_button(s, "pt-mode-both", ApplyMode::IndexAndTree))
                        .child(mode_button(s, "pt-mode-tree", ApplyMode::WorkingTree))
                        .child(mode_button(s, "pt-mode-index", ApplyMode::IndexOnly)),
                )
                .child(
                    crate::widgets::toggle(
                        s,
                        "pt-3way",
                        "Repli à trois voies : les conflits sont laissés dans les fichiers",
                        screen.three_way,
                    )
                    .on_press(|s: &mut State| s.patch_toggle_three_way()),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "pt-check", "Vérifier", false)
                                .on_press(|s: &mut State| s.patch_check()),
                        )
                        .child(
                            action(s, "pt-apply", "Appliquer", true)
                                .on_press(|s: &mut State| s.patch_apply()),
                        ),
                );
        }
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
