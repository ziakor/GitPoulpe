use gitpoulpe_git::ApplyMode;
use rune::{El, ScrollSpec, col, row, text};

use crate::patch_import::mode_caption;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

fn mode_button(s: &State, key: &str, mode: ApplyMode) -> El {
    action(s, key, mode_caption(mode), s.patches.mode == mode)
        .on_press(move |s: &mut State| s.patch_set_mode(mode))
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
            text(if screen.name.is_empty() {
                "Importer un patch".to_owned()
            } else {
                format!("Importer un patch · {}", screen.name)
            })
            .key("pt-title")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            action(s, "pt-pick", "Choisir un fichier…", true)
                .on_press_tx(|s: &mut State, tx| s.patch_pick(tx)),
        )
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
            text("Lecture du patch…".to_owned())
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
                text(format!("{} fichier(s) ciblé(s)", preview.files.len()))
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
                    action(
                        s,
                        "pt-3way",
                        if screen.three_way {
                            "[x] Repli à trois voies : les conflits sont laissés dans les fichiers"
                        } else {
                            "[ ] Repli à trois voies : les conflits sont laissés dans les fichiers"
                        },
                        false,
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
