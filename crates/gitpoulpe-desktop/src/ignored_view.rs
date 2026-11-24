use std::borrow::Cow;

use gitpoulpe_core::{IgnoreReason, IgnoreScope, IgnoreSource};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::ignored::{IgnoreTarget, IgnoredState, ReasonState, scope_caption};
use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 28.0;

fn path_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(path) = s.ignored.paths.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let name = path.display().to_string();
    let selected = s.ignored.target.as_ref().is_some_and(|t| {
        t.path.as_path() == path.components().collect::<std::path::PathBuf>().as_path()
    });
    let target = path.clone();
    el().key(format!("ip{index}"))
        .role("option")
        .label(name.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(12.0)
        .center_y()
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_ignored(target.clone()))
        .child(text(name).font(theme::PATH_SMALL).color(p.text).truncate())
}

fn action(s: &State, key: String, caption: String, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key)
        .role("button")
        .label(caption.clone())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .min_h(32.0)
        .px(12.0)
        .py(6.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .child(text(caption).font(theme::BODY).color(ink).wrap())
}

fn explain(reason: &IgnoreReason) -> String {
    let place = match &reason.source {
        IgnoreSource::Scope(scope) => scope_caption(scope),
        IgnoreSource::Other(file) => file.clone(),
    };
    let rule = format!(
        "Règle « {} », ligne {} de {place}.",
        reason.pattern, reason.line
    );
    if reason.tracked {
        format!(
            "{rule}\nCe fichier est déjà suivi par Git : une règle d'exclusion ne le retire pas de l'index. Pour qu'il ne soit plus suivi, il faut le désindexer explicitement (git rm --cached) ; GitPoulpe ne le fait jamais à votre place."
        )
    } else if reason.negated {
        format!("{rule}\nC'est une exception : le fichier est réintégré, il n'est pas ignoré.")
    } else {
        format!("{rule}\nLe fichier est ignoré.")
    }
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .w(380.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line);
    let Some(IgnoreTarget {
        path,
        is_dir,
        reason,
    }) = s.ignored.target.as_ref()
    else {
        return panel.child(
            text(
                "Choisissez un fichier ignoré, ou « Ignorer… » depuis un fichier non suivi."
                    .to_owned(),
            )
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    };
    let shown = if *is_dir {
        format!("{}/", path.display())
    } else {
        path.display().to_string()
    };
    panel = panel.child(
        text(shown)
            .key("ignore-target")
            .font(theme::PATH_SMALL)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    let known = match reason {
        ReasonState::Loading => {
            return panel.child(
                text("Lecture de la règle…".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted),
            );
        }
        ReasonState::Failed(message) => {
            return panel.child(
                text(format!("Lecture impossible : {message}"))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
        ReasonState::Known(known) => known,
    };
    panel = panel.child(
        text(match known {
            Some(reason) => explain(reason),
            None => "Aucune règle ne concerne ce chemin.".to_owned(),
        })
        .key("ignore-reason")
        .font(theme::SMALL)
        .color(p.muted)
        .wrap(),
    );
    if let Some(reason) = known
        && let IgnoreSource::Scope(_) = reason.source
    {
        panel = panel.child(
            action(
                s,
                "ignore-remove".to_owned(),
                format!("Retirer la règle « {} »", reason.pattern),
                false,
            )
            .on_press(|s: &mut State| s.ignore_remove()),
        );
    }
    let covered = known.as_ref().is_some_and(|r| r.excludes());
    if !covered {
        panel = panel.child(
            text("Ajouter une règle dans :".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        );
        for (i, scope) in IgnoredState::scopes_for(path).into_iter().enumerate() {
            let rule = gitpoulpe_core::rule_for_path(path, *is_dir, &scope).ok();
            let caption = match (&scope, &rule) {
                (_, Some(rule)) => format!("{} : {rule}", scope_caption(&scope)),
                _ => scope_caption(&scope),
            };
            let chosen: IgnoreScope = scope.clone();
            panel = panel.child(
                action(s, format!("ignore-add-{i}"), caption, false)
                    .on_press(move |s: &mut State| s.ignore_add(chosen.clone())),
            );
        }
    }
    panel
}

pub fn ignored_screen(s: &State) -> El {
    let p = s.palette;
    let context = if s.ignored.loading {
        "Lecture…".to_owned()
    } else {
        crate::widgets::counted(s.ignored.paths.len(), "fichier ignoré", "fichiers ignorés")
    };
    let header =
        crate::widgets::screen_head(s, "ignored-head", "Fichiers ignorés", context, Vec::new());
    let list: El = if let Some(error) = &s.ignored.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.ignored.paths.is_empty() && !s.ignored.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun fichier ignoré dans ce dépôt.".to_owned())
                .key("ignored-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("ignored-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("ignored-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.ignored.version, s.dark))
                    .virtual_rows(
                        s.ignored.paths.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("ik{i}")),
                        path_row,
                    ),
            )
    };
    let mut body = col().w_full().grow(1.0).min_h(0.0).child(list);
    if s.ignored.truncated {
        body = body.child(
            text(format!(
                "Liste tronquée aux {} premiers chemins.",
                gitpoulpe_git::MAX_IGNORED_ENTRIES
            ))
            .font(theme::SMALL)
            .color(p.muted)
            .px(20.0)
            .py(6.0),
        );
    }
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(body)
            .child(detail(s)),
    )
}
