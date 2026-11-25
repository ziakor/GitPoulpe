use std::borrow::Cow;

use gitpoulpe_core::{ConflictKind, Resolution, Version};
use gitpoulpe_git::ConflictSide;
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, screen_head};

const ROW_HEIGHT: f32 = 44.0;
const PREVIEW_LINES: usize = 60;

pub fn kind_caption(kind: ConflictKind) -> &'static str {
    match kind {
        ConflictKind::BothModified => "modifié des deux côtés",
        ConflictKind::BothAdded => "ajouté des deux côtés",
        ConflictKind::DeletedByUs => "supprimé de votre côté, modifié de l'autre",
        ConflictKind::DeletedByThem => "modifié de votre côté, supprimé de l'autre",
        ConflictKind::AddedByUs => "ajouté de votre côté seulement",
        ConflictKind::AddedByThem => "ajouté de l'autre côté seulement",
        ConflictKind::BothDeleted => "supprimé des deux côtés",
    }
}

fn state_caption(state: Resolution) -> &'static str {
    match state {
        Resolution::Unresolved => "en conflit",
        Resolution::Resolved => "résolu",
        Resolution::Stale => "modifié après résolution : à valider de nouveau",
    }
}

fn disabled(s: &State, key: &str, caption: &str) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("button")
        .label(caption.to_owned())
        .min_h(32.0)
        .px(12.0)
        .py(6.0)
        .center()
        .radius(3.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            text(caption.to_owned())
                .font(theme::BODY)
                .color(p.muted)
                .wrap(),
        )
}

fn file_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let rows = s.conflicts.rows();
    let Some(path) = rows.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let state = s.conflicts.state_of(path);
    let kind = s
        .conflicts
        .files
        .iter()
        .find(|f| &f.path == path)
        .map_or("", |f| kind_caption(f.kind));
    let selected = s.conflicts.selected.as_ref() == Some(path);
    let target = path.clone();
    let detail = if kind.is_empty() {
        state_caption(state).to_owned()
    } else {
        format!("{} · {kind}", state_caption(state))
    };
    el().key(format!("cf-row{index}"))
        .role("option")
        .label(path.display().to_string())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_conflict(target.clone()))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(path.display().to_string())
                        .font(theme::PATH_SMALL)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(if state == Resolution::Resolved {
                            p.muted
                        } else {
                            p.red
                        })
                        .truncate(),
                ),
        )
}

fn preview(s: &State, key: &str, title: String, version: Option<&Version>) -> El {
    let p = s.palette;
    let body = match version {
        None => "Aucune version de ce côté.".to_owned(),
        Some(v) => match &v.text {
            None => format!(
                "Fichier binaire ou trop volumineux : {} octets, objet {}.",
                v.size,
                v.oid.chars().take(10).collect::<String>()
            ),
            Some(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let mut shown = lines
                    .iter()
                    .take(PREVIEW_LINES)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                if lines.len() > PREVIEW_LINES {
                    shown.push_str(&format!(
                        "\n… {} de plus",
                        counted(lines.len() - PREVIEW_LINES, "ligne", "lignes")
                    ));
                }
                shown
            }
        },
    };
    col()
        .key(key.to_owned())
        .w_full()
        .gap(4.0)
        .child(text(title).font(theme::SMALL).color(p.muted).wrap())
        .child(
            text(body)
                .font(theme::PATH_SMALL)
                .color(p.text)
                .wrap()
                .selectable(true),
        )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let c = &s.conflicts;
    let mut panel = col()
        .key("cf-panel")
        .w(520.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(path) = c.selected.clone() else {
        return panel.child(
            text(if c.files.is_empty() {
                "Aucun fichier en conflit.".to_owned()
            } else {
                "Choisissez un fichier.".to_owned()
            })
            .font(theme::SMALL)
            .color(p.muted),
        );
    };
    let (ours, theirs) = c.operation.as_ref().map_or_else(
        || ("vous".to_owned(), "l'autre côté".to_owned()),
        |o| (o.ours_label.clone(), o.theirs_label.clone()),
    );
    panel = panel.child(
        text(path.display().to_string())
            .key("cf-selected")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    match c.state_of(&path) {
        Resolution::Resolved => {
            return panel.child(
                text("Ce fichier est résolu. Modifiez-le dans un éditeur si besoin : il devra alors être validé de nouveau.".to_owned())
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
        }
        Resolution::Stale => {
            return panel
                .child(
                    text("Ce fichier a été modifié après sa résolution : la continuation reste bloquée tant qu'il n'est pas validé de nouveau.".to_owned())
                        .key("cf-stale")
                        .font(theme::SMALL)
                        .color(p.red)
                        .wrap(),
                )
                .child(
                    action(s, "cf-revalidate", "Valider le contenu actuel", true)
                        .on_press(|s: &mut State| s.conflict_revalidate()),
                );
        }
        Resolution::Unresolved => {}
    }
    let Some(v) = &c.versions else {
        return panel.child(
            text("Lecture des versions…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    panel = panel
        .child(
            text(format!(
                "Conflit {} : {ours} contre {theirs}.",
                kind_caption(v.file.kind)
            ))
            .key("cf-kind")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(preview(
            s,
            "cf-base",
            "Version commune (base)".to_owned(),
            v.base.as_ref(),
        ))
        .child(preview(
            s,
            "cf-ours",
            format!("Version de {ours}"),
            v.ours.as_ref(),
        ))
        .child(preview(
            s,
            "cf-theirs",
            format!("Version de {theirs}"),
            v.theirs.as_ref(),
        ));
    let take_ours = if v.ours.is_some() {
        format!("Garder la version de {ours}")
    } else {
        format!("Accepter la suppression ({ours})")
    };
    let take_theirs = if v.theirs.is_some() {
        format!("Garder la version de {theirs}")
    } else {
        format!("Accepter la suppression ({theirs})")
    };
    panel = panel
        .child(
            action(s, "cf-take-ours", &take_ours, false)
                .on_press(|s: &mut State| s.conflict_take(ConflictSide::Ours)),
        )
        .child(
            action(s, "cf-take-theirs", &take_theirs, false)
                .on_press(|s: &mut State| s.conflict_take(ConflictSide::Theirs)),
        );
    if v.file.binary {
        return panel.child(
            text("Fichier binaire : choisissez l'une des deux versions, il n'y a pas de résultat à éditer.".to_owned())
                .key("cf-binary")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    if v.merged.is_none() && v.ours.is_none() && v.theirs.is_none() {
        return panel;
    }
    panel = panel
        .child(
            text("Résultat (éditable)".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        )
        .child(
            text_input(&c.result, theme::PATH_SMALL)
                .key("cf-result")
                .label("Résultat de la résolution")
                .w_full()
                .h(220.0)
                .px(10.0)
                .py(8.0)
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .color(p.text)
                .focus_ring(2.0, 2.0, p.accent)
                .on_edit(|s: &mut State| &mut s.conflicts.result)
                .on_input_key(|_: &mut State, event, tx| {
                    if event.key == rune::Key::Escape {
                        tx.blur();
                        return true;
                    }
                    false
                }),
        );
    if c.markers_pending {
        panel = panel
            .child(
                text(
                    "Le résultat contient encore des marqueurs de conflit (<<<<<<< ou >>>>>>>)."
                        .to_owned(),
                )
                .key("cf-markers")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            )
            .child(
                action(
                    s,
                    "cf-validate-anyway",
                    "Valider quand même avec les marqueurs",
                    false,
                )
                .on_press(|s: &mut State| s.conflict_validate_result(true)),
            );
    }
    panel.child(
        action(s, "cf-validate", "Valider ce résultat", true)
            .on_press(|s: &mut State| s.conflict_validate_result(false)),
    )
}

fn footer(s: &State) -> El {
    let p = s.palette;
    let c = &s.conflicts;
    let caption = c
        .operation
        .as_ref()
        .map_or("l'opération", |o| o.kind.caption());
    let mut bar = col()
        .w_full()
        .px(20.0)
        .py(10.0)
        .gap(8.0)
        .border(1.0, p.line);
    if c.confirm_abort {
        let saved = c
            .states
            .iter()
            .filter(|(_, st)| *st != Resolution::Unresolved)
            .count();
        bar = bar.child(
            col()
                .key("cf-abort-confirm")
                .w_full()
                .gap(6.0)
                .px(12.0)
                .py(10.0)
                .radius(3.0)
                .border(1.0, p.red)
                .child(
                    text(format!(
                        "Abandonner la {caption} rétablit l'état d'avant{}. Les fichiers sont d'abord copiés dans .git/gitpoulpe-backup ; les fichiers non suivis sont conservés.",
                        if saved == 0 {
                            String::new()
                        } else {
                            format!(
                                " : {}",
                                counted(
                                    saved,
                                    "résolution déjà faite sera défaite",
                                    "résolutions déjà faites seront défaites"
                                )
                            )
                        }
                    ))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "cf-abort-confirm-yes", "Abandonner", true)
                                .on_press(|s: &mut State| s.conflict_abort()),
                        )
                        .child(
                            action(s, "cf-abort-confirm-no", "Annuler", false)
                                .on_press(|s: &mut State| s.conflict_cancel_abort()),
                        ),
                ),
        );
    }
    if c.operation.as_ref().map(|o| o.kind) == Some(gitpoulpe_core::OperationKind::StashApply) {
        return bar.child(
            text(
                "Le stash est conservé. Une fois les fichiers résolus, ils sont simplement indexés ; supprimez le stash vous-même depuis la réserve quand vous n'en avez plus besoin."
                    .to_owned(),
            )
            .key("cf-stash-hint")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    let go = if c.ready() {
        action(s, "cf-continue", &format!("Continuer la {caption}"), true)
            .on_press(|s: &mut State| s.conflict_continue())
    } else {
        disabled(s, "cf-continue-off", &format!("Continuer la {caption}"))
    };
    let hint = if c.ready() {
        "Tous les fichiers sont résolus.".to_owned()
    } else {
        format!(
            "Il reste {} à résoudre ou à valider.",
            counted(c.remaining(), "fichier", "fichiers")
        )
    };
    let skippable = c.operation.as_ref().is_some_and(|o| {
        matches!(
            o.kind,
            gitpoulpe_core::OperationKind::CherryPick
                | gitpoulpe_core::OperationKind::Revert
                | gitpoulpe_core::OperationKind::Rebase
        )
    });
    let skip = skippable.then(|| {
        action(s, "cf-skip", "Passer ce commit", false).on_press(|s: &mut State| s.conflict_skip())
    });
    bar.child(
        row()
            .w_full()
            .center_y()
            .gap(12.0)
            .child(
                text(hint)
                    .key("cf-hint")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .grow(1.0)
                    .wrap(),
            )
            .children(skip)
            .child(go)
            .child(
                action(s, "cf-abort", &format!("Abandonner la {caption}"), false)
                    .on_press(|s: &mut State| s.conflict_ask_abort()),
            ),
    )
}

pub fn conflicts_screen(s: &State) -> El {
    let p = s.palette;
    let c = &s.conflicts;
    let context = match &c.operation {
        Some(op) => format!(
            "{} · {} à traiter",
            op.kind.caption(),
            counted(c.remaining(), "fichier", "fichiers")
        ),
        None if c.loading => "Lecture…".to_owned(),
        None => "Aucune opération en cours".to_owned(),
    };
    let header = screen_head(
        s,
        "conflicts-head",
        "Résoudre les conflits",
        context,
        vec![
            action(s, "cf-external-merge", "Outil de fusion externe", false).on_press(
                |s: &mut State| {
                    if let Some(path) = s.conflicts.selected.clone() {
                        s.tool_open_merge(path.to_string_lossy().into_owned());
                    }
                },
            ),
            action(s, "cf-tools", "Outils…", false).on_press(|s: &mut State| s.open_tools_config()),
        ],
    );
    let rows = c.rows();
    let list: El = if let Some(error) = &c.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else {
        col()
            .key("conflicts-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("conflicts-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((c.version, s.dark))
                    .virtual_rows(
                        rows.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("cfk{i}")),
                        file_row,
                    ),
            )
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(
            col()
                .w_full()
                .px(20.0)
                .children(crate::external_tools_view::tool_status(s)),
        )
        .child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(list)
                .child(detail(s)),
        )
        .child(footer(s))
}
