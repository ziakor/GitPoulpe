use std::borrow::Cow;
use std::path::Path;

use gitpoulpe_core::{
    ChangeEntry, DiffLine, DiffLineKind, DocumentKind, EntryKind, SplitIndex, StatusCode,
    display_label,
};
use rune::{
    Align, Color, Cursor, Cx, El, GrowthSpec, Key, Phase, ScrollSpec, Tx, col, el, row, text,
    text_input,
};

use crate::state::{DiffMode, LIST_WIDTH_DEFAULT, Screen, State, ViewKind, WatchStatus};
use crate::theme::{self, Palette};

const LIST_ROW_HEIGHT: f32 = 56.0;
pub const LINE_HEIGHT: f32 = 25.0;
const GUTTER_WIDTH: f32 = 128.0;

pub fn view(s: &State, _: &mut Cx<State>) -> El {
    let p = s.palette;
    let body = match &s.screen {
        Screen::Ready => workspace(s),
        Screen::Welcome => message_screen(
            &p,
            "Ouvrir un dépôt Git",
            "Choisissez un dossier, ou lancez gitpoulpe avec le chemin du dépôt.",
            true,
        ),
        Screen::Opening(path) => message_screen(&p, "Ouverture…", &display_label(path), false),
        Screen::Failed { path, message } => {
            let detail = if path.as_os_str().is_empty() {
                message.clone()
            } else {
                format!("{}\n{message}", display_label(path))
            };
            message_screen(&p, "Impossible d'ouvrir ce dépôt", &detail, true)
        }
    };
    let body = if s.profiles_open {
        crate::profiles_view::profiles_screen(s)
    } else {
        body
    };
    col()
        .w_full()
        .h_full()
        .fill(p.bg)
        .on_layout(|s: &mut State, rect| s.window_width = rect[2])
        .on_window_focus(|s: &mut State, focused, _| {
            if focused {
                s.refresh_on_focus();
            }
        })
        .child(top_bar(s))
        .child(body)
        .child(footer(s))
}

fn footer(s: &State) -> El {
    let p = s.palette;
    if s.screen != Screen::Ready {
        return el().h(0.0);
    }
    let message = s.notice.clone().unwrap_or_else(|| match &s.watch {
        WatchStatus::Unavailable(reason) => format!(
            "Surveillance indisponible ({reason}) : utilisez Actualiser ou revenez sur la fenêtre."
        ),
        _ => HINTS.to_owned(),
    });
    row()
        .w_full()
        .h(28.0)
        .px(20.0)
        .center_y()
        .fill(p.panel)
        .border(1.0, p.line)
        .child(text(message).font(theme::SMALL).color(p.muted).truncate())
}

const HINTS: &str = "/ filtrer · j et k : fichier suivant ou précédent · 1 et 2 : changer de vue · m : unifié ou séparé · ↑ ↓ ← → PgUp PgDn : lire · r : actualiser · s : indexer · u : désindexer · x : sélectionner · Maj+S : appliquer à la sélection · clic sur un numéro de ligne : sélectionner (Maj pour étendre) · Ctrl+O : ouvrir un dépôt";

fn open_button(p: &Palette, primary: bool) -> El {
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key("open")
        .role("button")
        .label("Ouvrir un dépôt")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .on_press_tx(|_: &mut State, tx: &mut Tx| crate::keys::open_dialog(tx))
        .child(text("Ouvrir un dépôt…").font(theme::BODY_STRONG).color(ink))
}

fn top_bar(s: &State) -> El {
    let p = s.palette;
    let mut context = col().gap(2.0).grow(1.0).min_w(0.0);
    if let Some(repo) = &s.repo {
        let branch = repo.snapshot.branch.head.clone().unwrap_or_default();
        context = context
            .child(
                text(repo.name.clone())
                    .font(theme::BODY_STRONG)
                    .color(p.text)
                    .truncate(),
            )
            .child(text(branch).font(theme::SMALL).color(p.accent).truncate());
    }
    let theme_toggle = el()
        .key("theme")
        .role("button")
        .label("Changer de thème")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| s.set_dark(!s.dark))
        .child(
            text(if s.dark {
                "Thème clair"
            } else {
                "Thème sombre"
            })
            .font(theme::BODY)
            .color(p.muted),
        );
    row()
        .w_full()
        .h(60.0)
        .px(20.0)
        .gap(20.0)
        .center_y()
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            row()
                .w(172.0)
                .child(text("git").font(theme::WORDMARK).color(p.text))
                .child(text("poulpe").font(theme::WORDMARK_BOLD).color(p.text)),
        )
        .child(context)
        .child(open_button(&p, false))
        .child(profile_button(s))
        .child(refresh_button(s))
        .child(theme_toggle)
}

fn profile_button(s: &State) -> El {
    let p = s.palette;
    let compact = s.window_width < 900.0;
    let caption = match (s.repo.is_some(), s.active_profile()) {
        _ if compact => "Profils".to_owned(),
        (true, Some(profile)) => format!("Profil : {}", profile.label),
        (true, None) => "Profil : configuration Git".to_owned(),
        (false, _) => "Profils".to_owned(),
    };
    el().key("profiles")
        .role("button")
        .label("Profils Git")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| s.open_profiles())
        .max_w(260.0)
        .child(text(caption).font(theme::BODY).color(p.muted).truncate())
}

fn refresh_button(s: &State) -> El {
    let p = s.palette;
    el().key("refresh")
        .role("button")
        .label("Actualiser")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| s.refresh())
        .child(text("Actualiser").font(theme::BODY).color(p.muted))
}

fn message_screen(p: &Palette, title: &str, detail: &str, with_open: bool) -> El {
    let mut card = col()
        .gap(10.0)
        .max_w(520.0)
        .child(text(title.to_owned()).font(theme::TITLE).color(p.text));
    card = card.child(
        text(detail.to_owned())
            .font(theme::BODY)
            .color(p.muted)
            .wrap(),
    );
    if with_open {
        card = card.child(open_button(p, true));
    }
    col().w_full().grow(1.0).center().child(card)
}

fn divider(s: &State) -> El {
    let p = s.palette;
    el().key("divider")
        .role("separator")
        .label("Largeur de la liste des fichiers")
        .focusable(true)
        .focus_ring(2.0, 0.0, p.accent)
        .w(6.0)
        .h_full()
        .cursor(Cursor::ResizeColumn)
        .hover(p.hover)
        .on_drag(|s: &mut State, drag, _| match drag.phase {
            Phase::Down if drag.clicks >= 2 => s.resize_list(LIST_WIDTH_DEFAULT),
            Phase::Down => drag.data = s.list_width,
            Phase::Move => s.resize_list(drag.data + drag.dx),
            Phase::Up if drag.cancelled() => s.resize_list(drag.data),
            Phase::Up => {}
        })
        .on_drag_end(|s: &mut State, _, _| s.save_list_width())
        .on_key(|s: &mut State, event, _| {
            let step = if event.mods.shift { 64.0 } else { 16.0 };
            match event.key {
                Key::Left => s.resize_list(s.list_width - step),
                Key::Right => s.resize_list(s.list_width + step),
                Key::Home => s.resize_list(LIST_WIDTH_DEFAULT),
                _ => return false,
            }
            s.save_list_width();
            true
        })
}

fn workspace(s: &State) -> El {
    row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(left_pane(s))
        .child(divider(s))
        .child(right_pane(s))
}

fn tab(s: &State, kind: ViewKind, label: &'static str) -> El {
    let p = s.palette;
    let active = s.view == kind;
    let (fill, ink, edge) = if active {
        (p.accent, p.on_accent, p.accent)
    } else {
        (p.raised, p.text, p.line)
    };
    el().key(label)
        .role("tab")
        .focus_selected(active)
        .label(format!("{label}, {} fichiers", s.count(kind)))
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .grow(1.0)
        .h(34.0)
        .radius(3.0)
        .fill(fill)
        .border(1.0, edge)
        .center()
        .hover(p.hover)
        .on_press(move |s: &mut State| s.set_view(kind))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(label).font(theme::BODY_STRONG).color(ink))
                .child(
                    text(s.count(kind).to_string())
                        .font(theme::SMALL)
                        .color(ink),
                ),
        )
}

fn view_hint(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Unstaged => {
            "Modifications du répertoire de travail, pas encore ajoutées à l'index."
        }
        ViewKind::Staged => {
            "Modifications ajoutées à l'index : elles feront partie du prochain commit."
        }
    }
}

fn left_pane(s: &State) -> El {
    let p = s.palette;
    let total = s.count(s.view);
    let filter = text_input(&s.filter, theme::BODY)
        .key("filter")
        .label("Rechercher un fichier")
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(format!("Nom ou chemin parmi {total} fichiers"), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.filter)
        .on_input_change(|s: &mut State, _| s.filter_changed())
        .on_input_key(|s: &mut State, event, tx| {
            if event.key != Key::Escape {
                return false;
            }
            if s.filter.text.is_empty() {
                tx.blur();
            } else {
                s.filter.text.clear();
                s.filter_changed();
            }
            true
        })
        .on_submit(|s: &mut State, tx: &mut Tx| {
            if let Some((index, _)) = s.entry_at(0) {
                s.jump_to(0);
                crate::keys::align_file(tx, index);
                tx.blur();
            }
        });
    let counter = if s.files.len() == total {
        format!("{total} fichiers")
    } else {
        format!("{} sur {total} fichiers", s.files.len())
    };
    let list = col()
        .key("list-rows")
        .role("listbox")
        .focus_group()
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((s.layout_version, s.view, s.current, s.marks_version))
        .virtual_rows(
            s.files.len(),
            LIST_ROW_HEIGHT,
            |s: &State, i| Cow::Owned(format!("f{}", s.files[i])),
            file_row,
        );
    col()
        .w(s.effective_list_width())
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            col()
                .w_full()
                .px(16.0)
                .py(12.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .role("tablist")
                        .focus_group()
                        .child(tab(s, ViewKind::Unstaged, "Non indexés"))
                        .child(tab(s, ViewKind::Staged, "Indexés")),
                )
                .child(
                    text(view_hint(s.view))
                        .font(theme::SMALL)
                        .color(p.muted)
                        .wrap(),
                )
                .child(
                    text("Rechercher un fichier")
                        .font(theme::SMALL)
                        .color(p.muted),
                )
                .child(filter)
                .child(text(counter).font(theme::SMALL).color(p.muted))
                .child(marked_bar(s)),
        )
        .child(
            col()
                .key("list-viewport")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

fn marked_bar(s: &State) -> El {
    if s.marked.is_empty() {
        return el().h(0.0);
    }
    let p = s.palette;
    let count = s.marked.len();
    let action = if s.view == ViewKind::Unstaged {
        "Indexer la sélection"
    } else {
        "Désindexer la sélection"
    };
    let caption = if count == 1 {
        "1 fichier sélectionné".to_owned()
    } else {
        format!("{count} fichiers sélectionnés")
    };
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(caption)
                .font(theme::SMALL)
                .color(p.muted)
                .grow(1.0)
                .truncate(),
        )
        .child(
            stage_button(
                s,
                "stage-marked".to_owned(),
                action.to_owned(),
                action.to_owned(),
                true,
            )
            .on_press(|s: &mut State| s.write_marked()),
        )
}

fn status_of(entry: &ChangeEntry, view: ViewKind) -> (&'static str, &'static str) {
    if entry.is_conflict() {
        return ("Conflit", "!");
    }
    if entry.kind == EntryKind::Untracked {
        return ("Non suivi", "?");
    }
    let code = if view == ViewKind::Staged {
        entry.index
    } else {
        entry.worktree
    };
    match code {
        StatusCode::Added => ("Ajouté", "A"),
        StatusCode::Deleted => ("Supprimé", "D"),
        StatusCode::Renamed => ("Renommé", "R"),
        StatusCode::Copied => ("Copié", "C"),
        StatusCode::TypeChanged => ("Type modifié", "T"),
        _ => ("Modifié", "M"),
    }
}

fn split_path(path: &Path) -> (String, String) {
    let name = path
        .file_name()
        .map(|n| display_label(Path::new(n)))
        .unwrap_or_default();
    let dir = path.parent().map(display_label).unwrap_or_default();
    (name, dir)
}

fn file_row(s: &State, position: usize) -> El {
    let p = s.palette;
    let Some((index, entry)) = s.entry_at(position) else {
        return el().h(LIST_ROW_HEIGHT);
    };
    let (name, dir) = split_path(&entry.path);
    let (word, letter) = status_of(entry, s.view);
    let selected = s.current == Some(position);
    let fill = if selected { p.selected } else { p.panel };
    el().key(format!("fr{index}"))
        .role("option")
        .focus_selected(selected)
        .label(format!("{}, {word}", display_label(&entry.path)))
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(LIST_ROW_HEIGHT)
        .fill(fill)
        .hover(p.hover)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
            s.jump_to(position);
            crate::keys::align_file(tx, index);
        })
        .child(
            row()
                .w_full()
                .h_full()
                .child(
                    el().w(if selected { 3.0 } else { 0.0 })
                        .h_full()
                        .fill(p.accent),
                )
                .child(mark_box(s, position, index))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .pr(16.0)
                        .py(8.0)
                        .gap(2.0)
                        .child(text(name).font(theme::BODY_MEDIUM).color(p.text).truncate())
                        .child(text(dir).font(theme::PATH_SMALL).color(p.muted).truncate()),
                )
                .child(
                    el().w(34.0).h_full().py(8.0).child(
                        text(letter)
                            .font(theme::SMALL)
                            .color(p.muted)
                            .w(34.0)
                            .align(Align::Center),
                    ),
                ),
        )
}

fn mark_box(s: &State, position: usize, index: usize) -> El {
    let p = s.palette;
    if !s.can_mark(position) {
        return el().w(16.0).h_full();
    }
    let path = s
        .entry_at(position)
        .map(|(_, e)| e.path.clone())
        .unwrap_or_default();
    let marked = s.marked.contains(&path);
    let label = format!("Sélectionner {} pour l'indexation", display_label(&path));
    el().w(40.0).h_full().center().child(
        el().key(format!("mk{index}"))
            .role("checkbox")
            .label(label)
            .w(18.0)
            .h(18.0)
            .radius(3.0)
            .center()
            .fill(if marked { p.accent } else { p.raised })
            .border(1.0, if marked { p.accent } else { p.line })
            .hover(p.hover)
            .on_press(move |s: &mut State| s.toggle_mark(position))
            .child(
                text(if marked { "✓" } else { "" })
                    .font(theme::TINY)
                    .color(p.on_accent),
            ),
    )
}

fn stage_button(s: &State, key: String, label: String, caption: String, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(28.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .child(text(caption).font(theme::SMALL).color(ink))
}

fn right_pane(s: &State) -> El {
    let p = s.palette;
    if s.files.is_empty() {
        let (title, detail) = if s.count(s.view) == 0 {
            (
                "Aucune modification",
                "Cette vue ne contient aucun fichier modifié.".to_owned(),
            )
        } else {
            (
                "Aucun fichier ne correspond",
                format!("Aucun résultat pour « {} ».", s.filter.text.trim()),
            )
        };
        return col()
            .grow(1.0)
            .h_full()
            .center()
            .child(message_card(&p, title, &detail));
    }
    let rows = col()
        .key("rows")
        .selection_group(true)
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((s.layout_version, s.view, s.dark, s.mode, s.pick_version))
        .virtual_rows(
            s.layout.total as usize,
            LINE_HEIGHT,
            |s: &State, i| Cow::Owned(row_key(s, i)),
            build_row,
        )
        .on_visible_rows(|s: &mut State, range| s.on_visible(range));
    let viewport = col()
        .key("viewport")
        .role("region")
        .label("Diffs")
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: s.stick_end,
        })
        .on_scroll_offset({
            let probe = s.scroll_probe.clone();
            let horizontal = s.scroll_x.clone();
            move |at| {
                probe.set([at.offset[1], at.max[1], at.viewport[1], 1.0]);
                horizontal.set([at.offset[0], at.max[0]]);
            }
        })
        .child(rows);
    let viewport = if s.mode == DiffMode::Unified {
        viewport.scroll_xy()
    } else {
        viewport
    };
    col()
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .child(diff_toolbar(s))
        .child(viewport)
}

fn mode_button(s: &State, mode: DiffMode, key: &'static str, label: &'static str) -> El {
    let p = s.palette;
    let active = s.mode == mode;
    let (fill, ink, edge) = if active {
        (p.accent, p.on_accent, p.accent)
    } else {
        (p.raised, p.text, p.line)
    };
    el().key(key)
        .role("tab")
        .label(format!("Affichage {label}"))
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .focus_selected(active)
        .h(28.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, edge)
        .hover(p.hover)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
            if s.mode != mode {
                s.toggle_mode(tx);
            }
        })
        .child(
            text(label)
                .font(if active {
                    theme::BODY_STRONG
                } else {
                    theme::BODY
                })
                .color(ink),
        )
}

fn diff_toolbar(s: &State) -> El {
    let p = s.palette;
    let position = match s.current {
        Some(current) => format!("Fichier {} sur {}", current + 1, s.files.len()),
        None => format!("{} fichiers", s.files.len()),
    };
    row()
        .w_full()
        .h(44.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .fill(p.bg)
        .border(1.0, p.line)
        .child(text(position).font(theme::BODY).color(p.muted).grow(1.0))
        .child(
            row()
                .gap(6.0)
                .role("tablist")
                .focus_group()
                .child(mode_button(s, DiffMode::Unified, "mode-unified", "Unifié"))
                .child(mode_button(s, DiffMode::Split, "mode-split", "Séparé")),
        )
}

fn message_card(p: &Palette, title: &str, detail: &str) -> El {
    col()
        .gap(8.0)
        .max_w(460.0)
        .child(
            text(title.to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            text(detail.to_owned())
                .font(theme::BODY)
                .color(p.muted)
                .wrap(),
        )
}

pub(crate) fn row_key(s: &State, i: usize) -> String {
    match s.locate(i as u64) {
        Some((position, 0)) => format!("h{}", s.files[position]),
        Some((position, local)) => format!("l{}:{}", s.files[position], local - 1),
        None => format!("x{i}"),
    }
}

fn build_row(s: &State, i: usize) -> El {
    let Some((position, local)) = s.locate(i as u64) else {
        return el().h(LINE_HEIGHT);
    };
    let Some((index, entry)) = s.entry_at(position) else {
        return el().h(LINE_HEIGHT);
    };
    if local == 0 {
        return header_row(s, index, entry);
    }
    body_row(s, index, local - 1)
}

fn header_row(s: &State, index: usize, entry: &ChangeEntry) -> El {
    let p = s.palette;
    let (word, _) = status_of(entry, s.view);
    let counts = s
        .doc(index)
        .and_then(|d| d.meta.as_ref())
        .filter(|m| m.added + m.removed > 0)
        .map(|m| (m.added, m.removed));
    let mut path = display_label(&entry.path);
    if let Some(original) = &entry.original_path {
        path = format!("{} → {path}", display_label(original));
    }
    col()
        .key(format!("h{index}"))
        .w_full()
        .py(6.0)
        .px(14.0)
        .child(
            col()
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .center_y()
                        .child(
                            text(path.clone())
                                .font(theme::PATH)
                                .color(p.text)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(header_action(s, index, entry, &path)),
                )
                .child({
                    let mut line = row()
                        .gap(0.0)
                        .child(text(word.to_owned()).font(theme::TINY).color(p.muted));
                    if let Some((added, removed)) = counts {
                        line = line
                            .child(text(" · ").font(theme::TINY).color(p.muted))
                            .child(text(format!("+{added}")).font(theme::TINY).color(p.green))
                            .child(text(" ").font(theme::TINY).color(p.muted))
                            .child(text(format!("−{removed}")).font(theme::TINY).color(p.red));
                    }
                    line
                }),
        )
}

fn header_action(s: &State, index: usize, entry: &ChangeEntry, label: &str) -> El {
    let staging = s.view == ViewKind::Unstaged;
    let eligible = if staging {
        entry.is_unstaged()
    } else {
        entry.is_staged()
    };
    if !eligible || entry.is_conflict() {
        return el().w(0.0);
    }
    let target = entry.path.clone();
    let lines_button = s
        .line_pick
        .as_ref()
        .filter(|pick| pick.index == index)
        .map(|_| {
            let verb = if staging { "Indexer" } else { "Désindexer" };
            stage_button(
                s,
                format!("sl{index}"),
                format!("{verb} les lignes sélectionnées de {label}"),
                format!("{verb} les lignes"),
                true,
            )
            .on_press(|s: &mut State| s.write_picked_lines())
        });
    let (key, verb) = if staging {
        (format!("st{index}"), "Indexer")
    } else {
        (format!("un{index}"), "Désindexer")
    };
    let file_button = stage_button(s, key, format!("{verb} {label}"), verb.to_owned(), false)
        .on_press(move |s: &mut State| {
            if s.view == ViewKind::Unstaged {
                s.stage_paths(vec![target.clone()]);
            } else {
                s.unstage_paths(vec![target.clone()]);
            }
        });
    row()
        .gap(6.0)
        .center_y()
        .child(lines_button.unwrap_or_else(|| el().w(0.0)))
        .child(file_button)
}

fn notice(s: &State, index: usize, key: String, message: String, tone: Tone) -> El {
    let p = s.palette;
    let ink = if tone == Tone::Error { p.red } else { p.muted };
    row()
        .key(key)
        .w_full()
        .px(28.0)
        .py(10.0)
        .child(text(message).font(theme::BODY).color(ink).wrap())
        .key(format!("n{index}"))
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tone {
    Plain,
    Error,
}

fn body_row(s: &State, index: usize, line: u64) -> El {
    let key = format!("l{index}:{line}");
    let Some(doc) = s.doc(index) else {
        return placeholder(s, key, "Chargement…");
    };
    if let Some(error) = &doc.error {
        return notice(
            s,
            index,
            key,
            format!("Lecture impossible : {error}"),
            Tone::Error,
        );
    }
    let Some(meta) = &doc.meta else {
        return placeholder(s, key, "Chargement…");
    };
    match &meta.kind {
        Some(DocumentKind::Text) | None => {
            if let Some(split) = s.split_index(index) {
                return split_row(s, index, line, split, key);
            }
            match s.line(index, line) {
                Some(diff_line) => line_row(
                    &s.palette,
                    key,
                    diff_line,
                    hunk_action(s, index, line),
                    pick_ui(s, index, line, diff_line),
                ),
                None => placeholder(s, key, "…"),
            }
        }
        Some(other) => notice(
            s,
            index,
            key,
            kind_message(other),
            if matches!(other, DocumentKind::Unreadable(_)) {
                Tone::Error
            } else {
                Tone::Plain
            },
        ),
    }
}

fn kind_message(kind: &DocumentKind) -> String {
    match kind {
        DocumentKind::Binary => "Fichier binaire : son contenu n'est pas affiché.".to_owned(),
        DocumentKind::Conflict => {
            "Conflit de fusion non résolu : le contenu n'est pas affiché.".to_owned()
        }
        DocumentKind::ModeChange { old, new } => {
            format!("Mode modifié : {old} → {new}, sans changement de contenu.")
        }
        DocumentKind::RenameOnly { from, to } => {
            format!(
                "Renommé de {} vers {}, sans changement de contenu.",
                display_label(from),
                display_label(to)
            )
        }
        DocumentKind::NoChanges => "Ce fichier n'a plus de changement dans cette vue.".to_owned(),
        DocumentKind::Unreadable(diagnostic) => format!("Lecture impossible : {diagnostic}"),
        DocumentKind::Text => String::new(),
    }
}

fn placeholder(s: &State, key: String, label: &str) -> El {
    row()
        .key(key)
        .w_full()
        .h(LINE_HEIGHT)
        .px(28.0)
        .center_y()
        .child(
            text(label.to_owned())
                .font(theme::BODY)
                .color(s.palette.muted),
        )
}

fn gutter(
    p: &Palette,
    old: Option<u32>,
    new: Option<u32>,
    sign: &'static str,
    ink: Color,
    fill: Color,
) -> El {
    let number = |value: Option<u32>| {
        text(value.map(|n| n.to_string()).unwrap_or_default())
            .font(theme::CODE)
            .color(p.muted)
            .w(46.0)
            .align(Align::End)
    };
    row()
        .w(GUTTER_WIDTH)
        .h(LINE_HEIGHT)
        .center_y()
        .fill(fill)
        .sticky_left(0.0)
        .child(el().w(14.0))
        .child(number(old))
        .child(number(new))
        .child(
            text(sign)
                .font(theme::CODE)
                .color(ink)
                .w(22.0)
                .align(Align::Center),
        )
}

fn partial_entry(s: &State, index: usize) -> Option<&ChangeEntry> {
    let entry = s.entries().get(index)?;
    if entry.kind != EntryKind::Ordinary || entry.is_conflict() {
        return None;
    }
    let allowed = match s.view {
        ViewKind::Unstaged => entry.worktree == StatusCode::Modified,
        ViewKind::Staged => matches!(entry.index, StatusCode::Modified | StatusCode::Added),
    };
    let text = s.doc(index)?.meta.as_ref()?.kind == Some(DocumentKind::Text);
    (allowed && text).then_some(entry)
}

fn pick_ui(s: &State, index: usize, doc_line: u64, line: &DiffLine) -> Option<(usize, u64, bool)> {
    if !matches!(line.kind, DiffLineKind::Added | DiffLineKind::Removed) {
        return None;
    }
    partial_entry(s, index).map(|_| (index, doc_line, s.is_picked(index, doc_line)))
}

fn hunk_action(s: &State, index: usize, doc_line: u64) -> Option<El> {
    let entry = partial_entry(s, index)?;
    let meta = s.doc(index)?.meta.as_ref()?;
    let hunk = meta.hunks.iter().position(|h| h.first_line == doc_line)?;
    let path = entry.path.clone();
    let verb = if s.view == ViewKind::Unstaged {
        "Indexer"
    } else {
        "Désindexer"
    };
    let label = format!("{verb} ce bloc de {}", display_label(&path));
    Some(
        el().h(LINE_HEIGHT).px(6.0).center_y().child(
            stage_button(
                s,
                format!("hb{index}:{hunk}"),
                label,
                format!("{verb} ce bloc"),
                false,
            )
            .h(LINE_HEIGHT - 4.0)
            .on_press(move |s: &mut State| s.write_hunk(path.clone(), hunk)),
        ),
    )
}

fn line_row(
    p: &Palette,
    key: String,
    line: &DiffLine,
    action: Option<El>,
    pick: Option<(usize, u64, bool)>,
) -> El {
    let text_key = format!("t{}", &key[1..]);
    let content = String::from_utf8_lossy(&line.text).into_owned();
    match line.kind {
        DiffLineKind::HunkHeader => row()
            .key(key)
            .w_full()
            .h(LINE_HEIGHT)
            .center_y()
            .fill(p.panel)
            .child(gutter(p, None, None, " ", p.muted, p.panel))
            .child(action.unwrap_or_else(|| el().w(0.0)))
            .child(text(content).font(theme::CODE).color(p.muted))
            .child(el().w(14.0)),
        DiffLineKind::NoNewlineAtEof => row()
            .key(key)
            .w_full()
            .h(LINE_HEIGHT)
            .center_y()
            .child(gutter(p, None, None, " ", p.muted, p.bg))
            .child(
                text("\\ Pas de saut de ligne à la fin du fichier")
                    .font(theme::CODE)
                    .color(p.muted),
            )
            .child(el().w(14.0)),
        kind => {
            let (fill, sign, ink) = match kind {
                DiffLineKind::Added => (Some(p.added), "+", p.green),
                DiffLineKind::Removed => (Some(p.removed), "−", p.red),
                _ => (None, " ", p.muted),
            };
            let picked = pick.is_some_and(|(_, _, selected)| selected);
            let base_fill = fill.unwrap_or(p.bg);
            let mut gutter_el = gutter(
                p,
                line.old_number,
                line.new_number,
                sign,
                ink,
                if picked { p.selected } else { base_fill },
            );
            if let Some((owner, doc_line, _)) = pick {
                gutter_el = gutter_el
                    .key(format!("gs{owner}:{doc_line}"))
                    .role("checkbox")
                    .label(format!(
                        "Sélectionner la ligne {doc_line} pour l'indexation"
                    ))
                    .hover(p.hover)
                    .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                        s.pick_line(owner, doc_line, tx.mods().shift);
                    });
            }
            let fill = if picked { Some(p.selected) } else { fill };
            let mut line_el = row()
                .key(key)
                .w_full()
                .h(LINE_HEIGHT)
                .center_y()
                .child(gutter_el)
                .child(
                    text(content)
                        .key(text_key)
                        .font(theme::CODE)
                        .color(p.text)
                        .selectable(true),
                )
                .child(el().w(14.0));
            if let Some(fill) = fill {
                line_el = line_el.fill(fill);
            }
            line_el
        }
    }
}

fn split_row(s: &State, index: usize, row_index: u64, split: &SplitIndex, key: String) -> El {
    let p = s.palette;
    let Some(cells) = split.cells(row_index) else {
        return placeholder(s, key, "…");
    };
    let left = cells.left.and_then(|l| s.line(index, l));
    let right = cells.right.and_then(|l| s.line(index, l));
    if (cells.left.is_some() && left.is_none()) || (cells.right.is_some() && right.is_none()) {
        return placeholder(s, key, "…");
    }
    if let (true, Some(line)) = (cells.full_width, left)
        && line.kind == DiffLineKind::HunkHeader
    {
        return line_row(
            &p,
            key,
            line,
            cells.left.and_then(|l| hunk_action(s, index, l)),
            None,
        );
    }
    row()
        .key(key)
        .w_full()
        .min_h(LINE_HEIGHT)
        .child(split_cell(
            &p,
            left,
            true,
            format!("t{index}:{row_index}:L"),
            left.zip(cells.left)
                .and_then(|(line, at)| pick_ui(s, index, at, line)),
        ))
        .child(el().w(1.0).fill(p.line))
        .child(split_cell(
            &p,
            right,
            false,
            format!("t{index}:{row_index}:R"),
            right
                .zip(cells.right)
                .and_then(|(line, at)| pick_ui(s, index, at, line)),
        ))
}

fn split_cell(
    p: &Palette,
    line: Option<&DiffLine>,
    old_side: bool,
    text_key: String,
    pick: Option<(usize, u64, bool)>,
) -> El {
    let cell = row().grow(1.0).min_w(0.0).px(8.0);
    let Some(line) = line else {
        return cell.fill(p.panel);
    };
    let number = |value: Option<u32>| {
        text(value.map(|n| n.to_string()).unwrap_or_default())
            .font(theme::CODE)
            .color(p.muted)
            .w(40.0)
            .align(Align::End)
    };
    if line.kind == DiffLineKind::NoNewlineAtEof {
        return cell.child(el().w(66.0)).child(
            text("\\ Pas de saut de ligne à la fin du fichier")
                .font(theme::CODE)
                .color(p.muted)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
    }
    let (fill, sign, ink) = match line.kind {
        DiffLineKind::Added => (Some(p.added), "+", p.green),
        DiffLineKind::Removed => (Some(p.removed), "−", p.red),
        _ => (None, " ", p.muted),
    };
    let numeral = if old_side {
        line.old_number
    } else {
        line.new_number
    };
    let picked = pick.is_some_and(|(_, _, selected)| selected);
    let fill = if picked { Some(p.selected) } else { fill };
    let mut gutter_el = row().child(number(numeral)).child(
        text(sign)
            .font(theme::CODE)
            .color(ink)
            .w(22.0)
            .align(Align::Center),
    );
    if let Some((owner, doc_line, _)) = pick {
        gutter_el = gutter_el
            .key(format!("gs{owner}:{doc_line}"))
            .role("checkbox")
            .label(format!(
                "Sélectionner la ligne {doc_line} pour l'indexation"
            ))
            .hover(p.hover)
            .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                s.pick_line(owner, doc_line, tx.mods().shift);
            });
    }
    let cell = cell.child(gutter_el).child(
        text(String::from_utf8_lossy(&line.text).into_owned())
            .key(text_key)
            .font(theme::CODE)
            .color(p.text)
            .selectable(true)
            .wrap()
            .grow(1.0)
            .min_w(0.0),
    );
    match fill {
        Some(fill) => cell.fill(fill),
        None => cell,
    }
}
