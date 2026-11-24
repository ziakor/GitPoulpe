use std::borrow::Cow;

use gitpoulpe_core::{FlowBranch, FlowKind, FlowPlan};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, field, screen_head};

const ROW_HEIGHT: f32 = 44.0;

fn branch_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(active): Option<&FlowBranch> =
        s.gitflow.view.as_ref().and_then(|v| v.active.get(index))
    else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.gitflow.selected.as_ref() == Some(&active.branch);
    let branch = active.branch.clone();
    let mut item = el()
        .key(format!("gf-row{index}"))
        .role("option")
        .label(active.branch.clone())
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .on_press(move |s: &mut State| s.select_flow_branch(branch.clone()));
    if selected {
        item = item.fill(p.panel);
    }
    item.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(
                text(active.branch.clone())
                    .font(theme::PATH_SMALL)
                    .color(p.text)
                    .truncate(),
            )
            .child(
                text(active.kind.caption().to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            ),
    )
}

fn plan_card(s: &State, plan: &FlowPlan, finish: bool) -> El {
    let p = s.palette;
    let mut card = col().key("gf-plan").w_full().gap(6.0).child(
        text(if finish {
            format!(
                "Terminer {} « {} » : étapes annoncées",
                plan.kind.caption(),
                plan.name
            )
        } else {
            format!(
                "Démarrer {} « {} » : étapes annoncées",
                plan.kind.caption(),
                plan.name
            )
        })
        .font(theme::BODY_STRONG)
        .color(p.text)
        .wrap(),
    );
    for (i, step) in plan.steps.iter().enumerate() {
        card = card.child(
            text(format!("{}. {}", i + 1, step.describe()))
                .key(format!("gf-step{i}"))
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    for (i, step) in plan.already_done.iter().enumerate() {
        card = card.child(
            text(format!("déjà fait : {}", step.describe()))
                .key(format!("gf-done{i}"))
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    card.child(
        action(s, "gf-confirm", "Exécuter ces étapes", true)
            .on_press(|s: &mut State| s.gitflow_confirm()),
    )
    .child(action(s, "gf-cancel", "Annuler", false).on_press(|s: &mut State| s.gitflow_dismiss()))
}

fn panel(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("gf-panel")
        .w(460.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(view) = &s.gitflow.view else {
        return panel.child(
            text("Lecture…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    if s.gitflow.busy {
        return panel.child(
            text("Exécution en cours…".to_owned())
                .key("gf-busy")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(done) = &s.gitflow.done {
        panel = panel.child(
            text(done.clone())
                .key("gf-ok")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &s.gitflow.error {
        panel = panel.child(
            text(error.clone())
                .key("gf-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some((plan, finish)) = &s.gitflow.plan {
        return panel.child(plan_card(s, plan, *finish));
    }
    panel = panel.child(
        text(if view.saved {
            "Convention enregistrée dans ce dépôt".to_owned()
        } else {
            "Convention par défaut : rien n'est enregistré dans ce dépôt tant que vous ne l'enregistrez pas".to_owned()
        })
        .key("gf-saved")
        .font(theme::BODY_STRONG)
        .color(p.text)
        .wrap(),
    );
    if !view.main_exists {
        panel = panel.child(
            text(format!(
                "La branche principale « {} » n'existe pas.",
                view.config.main
            ))
            .key("gf-main-missing")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    if !view.develop_exists {
        panel = panel.child(
            text(format!(
                "La branche de développement « {} » n'existe pas : l'enregistrement la créera à partir de la branche principale, après confirmation.",
                view.config.develop
            ))
            .key("gf-develop-missing")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    panel = panel
        .child(
            field(s, "gf-main", "Branche principale", &s.gitflow.main, "main")
                .on_edit(|s: &mut State| &mut s.gitflow.main),
        )
        .child(
            field(
                s,
                "gf-develop",
                "Branche de développement",
                &s.gitflow.develop,
                "develop",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.develop),
        )
        .child(
            field(
                s,
                "gf-feature",
                "Préfixe feature",
                &s.gitflow.feature,
                "feature/",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.feature),
        )
        .child(
            field(
                s,
                "gf-release",
                "Préfixe release",
                &s.gitflow.release,
                "release/",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.release),
        )
        .child(
            field(
                s,
                "gf-hotfix",
                "Préfixe hotfix",
                &s.gitflow.hotfix,
                "hotfix/",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.hotfix),
        )
        .child(
            field(
                s,
                "gf-tag",
                "Préfixe de tag",
                &s.gitflow.tag_prefix,
                "(aucun)",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.tag_prefix),
        );
    if let Some((config, create_develop)) = &s.gitflow.save_confirm {
        let mut announce = format!(
            "Seront écrites dans .git/config (gitflow.*) : principale {}, développement {}, préfixes {} {} {}, tag « {} ».",
            config.main,
            config.develop,
            config.feature,
            config.release,
            config.hotfix,
            config.tag_prefix
        );
        if *create_develop {
            announce.push_str(&format!(
                " La branche {} sera créée à partir de {}.",
                config.develop, config.main
            ));
        }
        panel = panel
            .child(
                text(announce)
                    .key("gf-save-announce")
                    .font(theme::SMALL)
                    .color(p.text)
                    .wrap(),
            )
            .child(
                action(s, "gf-save-confirm", "Enregistrer", true)
                    .on_press(|s: &mut State| s.gitflow_save_confirm()),
            )
            .child(
                action(s, "gf-save-cancel", "Annuler", false)
                    .on_press(|s: &mut State| s.gitflow_dismiss()),
            );
    } else {
        panel = panel.child(
            action(s, "gf-save", "Enregistrer la convention…", false)
                .on_press(|s: &mut State| s.gitflow_save_preview()),
        );
    }
    panel = panel
        .child(
            text("Démarrer".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "gf-name",
                "Nom du flux",
                &s.gitflow.name,
                "Nom (sans préfixe)",
            )
            .on_edit(|s: &mut State| &mut s.gitflow.name),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    action(s, "gf-start-feature", "Feature", false)
                        .on_press(|s: &mut State| s.gitflow_preview(FlowKind::Feature, false)),
                )
                .child(
                    action(s, "gf-start-release", "Release", false)
                        .on_press(|s: &mut State| s.gitflow_preview(FlowKind::Release, false)),
                )
                .child(
                    action(s, "gf-start-hotfix", "Hotfix", false)
                        .on_press(|s: &mut State| s.gitflow_preview(FlowKind::Hotfix, false)),
                ),
        );
    let selected = s
        .gitflow
        .selected
        .as_ref()
        .and_then(|b| view.active.iter().find(|a| &a.branch == b));
    if let Some(active) = selected {
        let kind = active.kind;
        panel = panel.child(
            action(s, "gf-finish", &format!("Terminer {}", active.branch), true)
                .on_press(move |s: &mut State| s.gitflow_preview(kind, true)),
        );
    }
    panel
}

pub fn gitflow_screen(s: &State) -> El {
    let p = s.palette;
    let count = s.gitflow.view.as_ref().map_or(0, |v| v.active.len());
    let context = if s.gitflow.loading && s.gitflow.view.is_none() {
        "Lecture…".to_owned()
    } else {
        counted(count, "branche de flux", "branches de flux")
    };
    let header = screen_head(s, "gf-head", "Gitflow", context, Vec::new());
    let list: El = if count == 0 {
        col().w_full().grow(1.0).center().child(
            text("Aucune branche feature, release ou hotfix en cours.".to_owned())
                .key("gf-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("gf-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("gf-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.gitflow.version, s.dark))
                    .virtual_rows(
                        count,
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("gfk{i}")),
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
            .child(panel(s)),
    )
}
