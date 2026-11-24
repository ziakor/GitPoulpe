use std::borrow::Cow;

use gitpoulpe_core::{Submodule, SubmoduleState};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, screen_head};

const ROW_HEIGHT: f32 = 52.0;

fn state_caption(state: SubmoduleState) -> &'static str {
    match state {
        SubmoduleState::NotInitialized => "non initialisé",
        SubmoduleState::Initialized => "initialisé",
        SubmoduleState::UpToDate => "sur le commit attendu",
        SubmoduleState::Differs => "sur un autre commit que celui attendu",
        SubmoduleState::Conflict => "en conflit",
    }
}

fn short(oid: &str) -> String {
    oid.chars().take(10).collect()
}

fn submodule_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(sub): Option<&Submodule> = s.submodules.list.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.submodules.selected.as_ref() == Some(&sub.path);
    let mut detail = format!(
        "{} · attendu {}",
        state_caption(sub.state),
        short(&sub.expected)
    );
    if let Some(current) = &sub.current {
        detail.push_str(&format!(" · actuel {}", short(current)));
    }
    if sub.dirty() {
        detail.push_str(&format!(
            " · {} modifié(s), {} non suivi(s)",
            sub.modified, sub.untracked
        ));
    }
    let indent = 16.0 + sub.depth as f32 * 16.0;
    let path = sub.path.clone();
    let mut row = el()
        .key(format!("sm-row{index}"))
        .role("option")
        .label(sub.path.display().to_string())
        .w_full()
        .h(ROW_HEIGHT)
        .pl(indent)
        .pr(16.0)
        .on_press(move |s: &mut State| s.select_submodule(path.clone()));
    if selected {
        row = row.fill(p.panel);
    }
    row.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(
                text(sub.path.display().to_string())
                    .font(theme::PATH_SMALL)
                    .color(p.text)
                    .truncate(),
            )
            .child(
                text(detail)
                    .font(theme::SMALL)
                    .color(if sub.state == SubmoduleState::UpToDate {
                        p.muted
                    } else {
                        p.red
                    })
                    .truncate(),
            ),
    )
}

fn panel(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("sm-panel")
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
    if let Some((phase, percent)) = &s.submodules.running {
        return panel
            .child(
                text(format!("{phase} {percent} %"))
                    .key("sm-running")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                action(s, "sm-cancel", "Annuler", false)
                    .on_press(|s: &mut State| s.submodule_cancel()),
            );
    }
    if let Some(done) = &s.submodules.done {
        panel = panel.child(
            text(done.clone())
                .key("sm-done")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &s.submodules.error {
        panel = panel.child(
            text(error.clone())
                .key("sm-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let selected = s
        .submodules
        .selected
        .as_ref()
        .and_then(|path| s.submodules.list.iter().find(|m| &m.path == path));
    let Some(sub) = selected else {
        return panel.child(
            text("Choisissez un submodule dans la liste.".to_owned())
                .key("sm-hint")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel.child(
        text(sub.name.clone())
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap(),
    );
    match (&sub.url, &sub.url_refused) {
        (_, Some(why)) => {
            panel = panel.child(
                text(format!("URL refusée : {why}"))
                    .key("sm-url-refused")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
        (Some(url), None) => {
            panel = panel.child(
                text(url.clone())
                    .key("sm-url")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
        }
        _ => {}
    }
    if sub.state != SubmoduleState::NotInitialized {
        panel = panel.child(
            text(format!(
                "En avance de {} commit(s), en retard de {}.",
                sub.ahead, sub.behind
            ))
            .key("sm-counts")
            .font(theme::SMALL)
            .color(p.muted),
        );
    }
    if sub.state == SubmoduleState::NotInitialized {
        panel = panel.child(
            action(s, "sm-init", "Initialiser", true)
                .on_press(|s: &mut State| s.submodule_run(true, false)),
        );
    } else {
        panel = panel.child(
            action(s, "sm-update", "Mettre sur le commit attendu", true)
                .on_press(|s: &mut State| s.submodule_run(false, false)),
        );
        if s.submodules.needs_decision.as_ref() == Some(&sub.path) {
            panel = panel
                .child(
                    text("Des commits locaux non publiés seront quittés ; un repère de récupération les garde.".to_owned())
                        .font(theme::SMALL)
                        .color(p.red)
                        .wrap(),
                )
                .child(
                    action(s, "sm-confirm", "Mettre à jour malgré tout", false)
                        .on_press(|s: &mut State| s.submodule_run(false, true)),
                );
        }
        panel = panel.child(
            action(s, "sm-open", "Ouvrir le submodule", false)
                .on_press(|s: &mut State| s.submodule_open()),
        );
    }
    panel
}

pub fn submodules_screen(s: &State) -> El {
    let p = s.palette;
    let context = if s.submodules.loading {
        "Lecture…".to_owned()
    } else {
        counted(s.submodules.list.len(), "sous-module", "sous-modules")
    };
    let header = screen_head(s, "sm-head", "Sous-modules", context, Vec::new());
    let list: El = if s.submodules.list.is_empty() && !s.submodules.loading {
        col().w_full().grow(1.0).center().child(
            text("Ce dépôt n'a pas de submodule.".to_owned())
                .key("sm-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("sm-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("sm-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.submodules.version, s.dark))
                    .virtual_rows(
                        s.submodules.list.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("smk{i}")),
                        submodule_row,
                    ),
            )
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(panel(s)),
    )
}
