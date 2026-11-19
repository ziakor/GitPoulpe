use std::borrow::Cow;

use gitpoulpe_git::{FileChange, FileRevision};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::file_history::PanelMode;
use crate::history_state::format_date;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

const ROW_HEIGHT: f32 = 52.0;
const MAX_LINES: usize = 3000;
const BLAME_ROW: f32 = 22.0;

fn change_label(revision: &FileRevision) -> String {
    match (&revision.change, &revision.old_path) {
        (FileChange::Added, _) => "ajouté".to_owned(),
        (FileChange::Modified, _) => "modifié".to_owned(),
        (FileChange::Deleted, _) => "supprimé".to_owned(),
        (FileChange::TypeChanged, _) => "type modifié".to_owned(),
        (FileChange::Renamed, Some(old)) => format!("renommé depuis {old}"),
        (FileChange::Copied, Some(old)) => format!("copié depuis {old}"),
        (FileChange::Renamed, None) => "renommé".to_owned(),
        (FileChange::Copied, None) => "copié".to_owned(),
    }
}

fn revision_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(revision) = s
        .file_history
        .history
        .as_ref()
        .and_then(|h| h.revisions.get(index))
    else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.file_history.selected.as_deref() == Some(revision.oid.as_str());
    let oid = revision.oid.clone();
    let moved = revision.path != s.file_history.path;
    let detail = format!(
        "{} · {} · {} UTC · {}{}",
        revision.oid.chars().take(8).collect::<String>(),
        revision.author,
        format_date(revision.timestamp),
        change_label(revision),
        if moved {
            format!(" · chemin : {}", revision.path)
        } else {
            String::new()
        }
    );
    el().key(format!("fh-row{index}"))
        .role("option")
        .label(revision.subject.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_file_revision(oid.clone()))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(revision.subject.clone())
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(if revision.change == FileChange::Deleted {
                            p.red
                        } else {
                            p.muted
                        })
                        .truncate(),
                ),
        )
}

fn code_lines(s: &State, mut panel: El, body: &str, prefix: &str) -> El {
    let p = s.palette;
    for (n, line) in body.lines().take(MAX_LINES).enumerate() {
        let mut item = row()
            .key(format!("{prefix}{n}"))
            .w_full()
            .px(6.0)
            .child(text(line.to_owned()).font(theme::CODE).color(p.text));
        if prefix == "fh-d" {
            if line.starts_with('+') && !line.starts_with("+++") {
                item = item.fill(p.added);
            } else if line.starts_with('-') && !line.starts_with("---") {
                item = item.fill(p.removed);
            }
        }
        panel = panel.child(item);
    }
    if body.lines().count() > MAX_LINES {
        panel = panel.child(
            text(format!(
                "Affichage limité à {MAX_LINES} lignes ; le reste n'est pas affiché."
            ))
            .key("fh-limit")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    panel
}

fn tabs(s: &State) -> El {
    let mode = s.file_history.mode;
    row()
        .gap(8.0)
        .child(
            action(s, "fh-show-diff", "Diff", mode == PanelMode::Diff)
                .on_press(|s: &mut State| s.set_file_history_mode(PanelMode::Diff)),
        )
        .child(
            action(s, "fh-show-content", "Contenu", mode == PanelMode::Content)
                .on_press(|s: &mut State| s.set_file_history_mode(PanelMode::Content)),
        )
        .child(
            action(s, "fh-show-blame", "Blame", mode == PanelMode::Blame)
                .on_press(|s: &mut State| s.set_file_history_mode(PanelMode::Blame)),
        )
}

fn blame_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(line) = s
        .file_history
        .blame
        .as_ref()
        .and_then(|b| b.lines.get(index))
    else {
        return el().h(BLAME_ROW);
    };
    let label = match &line.oid {
        Some(oid) => format!(
            "{} {:<14.14} {:>5} │ {}",
            oid.chars().take(8).collect::<String>(),
            line.author,
            line.line,
            line.text
        ),
        None => format!(
            "{:<8} {:<14} {:>5} │ {}",
            "local", "non enregistré", line.line, line.text
        ),
    };
    let mut item = row()
        .key(format!("fb-row{index}"))
        .w_full()
        .h(BLAME_ROW)
        .px(6.0)
        .center_y()
        .child(
            text(label.clone())
                .font(theme::CODE)
                .color(if line.oid.is_some() { p.text } else { p.accent })
                .truncate(),
        );
    match line.oid.clone() {
        Some(oid) => {
            item = item
                .role("button")
                .label(format!("Ouvrir le commit {} : {}", oid, line.summary))
                .hover(p.hover)
                .on_press(move |s: &mut State| s.blame_open_commit(oid.clone()));
        }
        None => item = item.fill(p.raised),
    }
    item
}

fn blame_panel(s: &State) -> El {
    let p = s.palette;
    let screen = &s.file_history;
    let mut panel = col()
        .w(560.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(tabs(s));
    if screen.blame_loading {
        return panel.child(
            text("Attribution des lignes…".to_owned())
                .key("fb-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(error) = &screen.blame_error {
        return panel.child(
            text(format!("Blame impossible : {error}"))
                .key("fb-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let Some(blame) = &screen.blame else {
        return panel;
    };
    let local = blame.lines.iter().filter(|l| l.oid.is_none()).count();
    let scope = match &blame.revision {
        Some(rev) => format!("révision {}", rev.chars().take(8).collect::<String>()),
        None => "fichier local (copie de travail)".to_owned(),
    };
    panel = panel.child(
        text(format!(
            "Blame de {} · {scope} · {} ligne(s){}",
            blame.path,
            blame.lines.len(),
            if local > 0 {
                format!(" dont {local} non enregistrée(s)")
            } else {
                String::new()
            }
        ))
        .key("fb-title")
        .font(theme::PATH_SMALL)
        .color(p.text)
        .wrap(),
    );
    if let Some(notice) = &screen.blame_notice {
        panel = panel.child(
            text(notice.clone())
                .key("fb-notice")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    panel.child(
        col()
            .key("fb-viewport")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("fb-rows")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((screen.version, s.dark))
                    .virtual_rows(
                        blame.lines.len(),
                        BLAME_ROW,
                        |_: &State, i| Cow::Owned(format!("fbk{i}")),
                        blame_row,
                    ),
            ),
    )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let screen = &s.file_history;
    if screen.mode == PanelMode::Blame {
        return blame_panel(s);
    }
    let mut panel = col()
        .key("fh-panel")
        .w(560.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(error) = &screen.revision_error {
        return panel.child(
            text(format!("Lecture impossible : {error}"))
                .key("fh-revision-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if screen.selected.is_none() {
        return panel.child(
            text(
                "Sélectionnez une révision pour lire le contenu du fichier et son diff.".to_owned(),
            )
            .key("fh-hint")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    if screen.revision_loading {
        return panel.child(
            text("Lecture…".to_owned())
                .key("fh-revision-loading")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    let Some(view) = &screen.revision else {
        return panel;
    };
    panel = panel
        .child(
            text(format!("{} · {}", view.oid, view.path))
                .key("fh-selected")
                .font(theme::PATH_SMALL)
                .color(p.text)
                .wrap()
                .selectable(true),
        )
        .child(tabs(s));
    if screen.mode == PanelMode::Content {
        let note = if view.deleted {
            Some("Le fichier est supprimé à cette révision : il n'a pas de contenu.")
        } else if view.binary {
            Some("Contenu binaire : non affiché.")
        } else if view.content_too_large {
            Some("Fichier de plus de 1 Mio : contenu non affiché.")
        } else {
            None
        };
        return match (&view.content, note) {
            (Some(content), _) => code_lines(s, panel, content, "fh-c"),
            (None, note) => panel.child(
                text(note.unwrap_or("Contenu indisponible.").to_owned())
                    .key("fh-no-content")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            ),
        };
    }
    if view.diff.is_empty() {
        panel = panel.child(
            text(if view.diff_truncated {
                "Le diff dépasse la limite de lecture : il n'est pas affiché.".to_owned()
            } else {
                "Aucun changement de contenu à afficher pour cette révision.".to_owned()
            })
            .key("fh-no-diff")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
        return panel;
    }
    panel = code_lines(s, panel, &view.diff, "fh-d");
    if view.diff_truncated {
        panel = panel.child(
            text("Diff tronqué : seule la première partie est affichée.".to_owned())
                .key("fh-diff-truncated")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    panel
}

pub fn file_history_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.file_history;
    let count = screen.history.as_ref().map_or(0, |h| h.revisions.len());
    let title = if screen.loading {
        format!("Historique de {} · lecture…", screen.path)
    } else {
        format!("Historique de {} · {count} révision(s)", screen.path)
    };
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(title)
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(s, "fh-blame-local", "Blame du fichier local", false)
                .on_press(|s: &mut State| s.file_blame_of_working_copy()),
        )
        .child(
            action(s, "fh-refresh", "Actualiser", false)
                .on_press(|s: &mut State| s.reload_file_history()),
        )
        .child(
            action(s, "fh-close", "Fermer", false).on_press(|s: &mut State| s.close_file_history()),
        );
    let mut notes = col().w_full().px(20.0).gap(4.0);
    if let Some(notice) = &screen.notice {
        notes = notes.child(
            text(notice.clone())
                .key("fh-notice")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(history) = &screen.history {
        if history.incomplete {
            notes = notes.child(
                text("Historique incomplet : ce dépôt est un clone superficiel, les révisions plus anciennes ne sont pas disponibles. Récupérez l'historique complet (git fetch --unshallow) pour les voir.".to_owned())
                    .key("fh-incomplete")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
        if history.truncated {
            notes = notes.child(
                text(format!(
                    "Liste limitée aux {} révisions les plus récentes.",
                    crate::file_history::HISTORY_LIMIT
                ))
                .key("fh-truncated")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        }
    }
    let list: El = if let Some(error) = &screen.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .key("fh-error")
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if count == 0 && !screen.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucune révision ne touche ce chemin.".to_owned())
                .key("fh-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("fh-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("fh-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((screen.version, s.dark))
                    .virtual_rows(
                        count,
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("fhk{i}")),
                        revision_row,
                    ),
            )
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(notes)
        .child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(list)
                .child(detail(s)),
        )
}
