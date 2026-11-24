use std::borrow::Cow;

use gitpoulpe_core::HookInfo;
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, screen_head};

const ROW_HEIGHT: f32 = 48.0;

fn status(hook: &HookInfo) -> &'static str {
    match (hook.known, hook.executable) {
        (true, true) => "actif",
        (true, false) => "désactivé (non exécutable : Git l'ignore)",
        (false, _) => "pas un hook Git",
    }
}

fn hook_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(hook) = s.hooks.view.as_ref().and_then(|v| v.hooks.get(index)) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.hooks.selected.as_deref() == Some(hook.name.as_str());
    let name = hook.name.clone();
    let detail = format!(
        "{} · {}{}",
        status(hook),
        hook.interpreter.as_deref().unwrap_or("sans interpréteur"),
        if hook.symlink {
            " · lien symbolique"
        } else {
            ""
        }
    );
    el().key(format!("hk-row{index}"))
        .role("option")
        .label(hook.name.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_hook(&name))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(hook.name.clone())
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(if hook.active() { p.muted } else { p.red })
                        .truncate(),
                ),
        )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("hk-panel")
        .w(440.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(view) = &s.hooks.view {
        panel = panel
            .child(
                text("Dossier effectif des hooks".to_owned())
                    .font(theme::BODY_STRONG)
                    .color(p.text),
            )
            .child(
                text(view.dir.display().to_string())
                    .key("hk-dir")
                    .font(theme::PATH_SMALL)
                    .color(p.text)
                    .wrap()
                    .selectable(true),
            );
        panel = panel.child(
            text(match (&view.configured_path, &view.configured_scope) {
                (Some(path), Some(scope)) => {
                    format!("core.hooksPath = {path} (portée : {scope})")
                }
                (Some(path), None) => format!("core.hooksPath = {path}"),
                _ => "Aucun core.hooksPath : dossier par défaut du dépôt.".to_owned(),
            })
            .key("hk-config")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
        if view.shared {
            panel = panel.child(
                text(if view.in_worktree {
                    "Ce dossier fait partie du projet : modifier un hook y change un fichier suivi."
                } else {
                    "Ce dossier est hors du dépôt : il peut servir à plusieurs dépôts."
                }
                .to_owned())
                .key("hk-shared")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        }
    }
    panel = panel.child(
        text("Un hook en échec garde sa cause et sa sortie, avec les secrets masqués ; rien n'est contourné (jamais --no-verify).".to_owned())
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
    );
    let Some(hook) = s
        .hooks
        .selected
        .as_deref()
        .and_then(|n| s.hooks.view.as_ref()?.hooks.iter().find(|h| h.name == n))
    else {
        return panel.child(
            text("Sélectionnez un hook.".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    panel = panel
        .child(el().h(1.0).w_full().fill(p.line))
        .child(
            text(hook.name.clone())
                .key("hk-selected")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!("{} · {} octets", status(hook), hook.size))
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    if hook.known && !hook.symlink {
        panel = panel.child(
            action(
                s,
                "hk-toggle",
                if hook.executable {
                    "Désactiver ce hook"
                } else {
                    "Activer ce hook"
                },
                true,
            )
            .on_press(|s: &mut State| s.hook_toggle(false)),
        );
    } else if hook.symlink {
        panel = panel.child(
            text("Lien symbolique : son activation n'est pas modifiée d'ici (la cible peut être partagée).".to_owned())
                .key("hk-symlink")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some((name, enabled)) = &s.hooks.confirm_shared
        && *name == hook.name
    {
        panel = panel.child(
            col()
                .key("hk-confirm")
                .w_full()
                .gap(6.0)
                .px(12.0)
                .py(10.0)
                .radius(3.0)
                .border(1.0, p.red)
                .child(
                    text(format!(
                        "Ce dossier est partagé : {} le hook {name} vaut pour tous les dépôts qui l'utilisent. Confirmez la portée.",
                        if *enabled { "activer" } else { "désactiver" }
                    ))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "hk-confirm-yes", "Confirmer pour tous", true)
                                .on_press(|s: &mut State| s.hook_toggle(true)),
                        )
                        .child(
                            action(s, "hk-confirm-no", "Annuler", false)
                                .on_press(|s: &mut State| s.hook_cancel_shared()),
                        ),
                ),
        );
    }
    panel
}

pub fn hooks_screen(s: &State) -> El {
    let p = s.palette;
    let count = s.hooks.view.as_ref().map_or(0, |v| v.hooks.len());
    let context = if s.hooks.loading {
        "Lecture…".to_owned()
    } else {
        counted(count, "fichier", "fichiers")
    };
    let header = screen_head(s, "hk-head", "Hooks", context, Vec::new());
    let list: El = if let Some(error) = &s.hooks.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if count == 0 && !s.hooks.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun hook dans le dossier effectif.".to_owned())
                .key("hooks-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("hooks-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("hooks-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.hooks.version, s.dark))
                    .virtual_rows(
                        count,
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("hkk{i}")),
                        hook_row,
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
