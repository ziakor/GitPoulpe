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
            None,
        ),
        Screen::Opening(path) => {
            message_screen(&p, "Ouverture…", &display_label(path), false, None)
        }
        Screen::Failed { path, message } => {
            let detail = if path.as_os_str().is_empty() {
                message.clone()
            } else {
                format!("{}\n{message}", display_label(path))
            };
            let here = (message == "Ce dossier n'est pas un dépôt Git.").then(|| path.clone());
            message_screen(&p, "Impossible d'ouvrir ce dépôt", &detail, true, here)
        }
    };
    let body = if s.overview.open {
        crate::overview_view::overview_screen(s)
    } else if s.favorites.open {
        crate::favorites_view::favorites_screen(s)
    } else if s.workspaces.open {
        crate::workspaces_view::workspaces_screen(s)
    } else if s.shortcuts.open {
        crate::shortcuts_view::shortcuts_screen(s)
    } else if s.tools.open {
        crate::external_tools_view::tools_screen(s)
    } else if s.clone.open {
        crate::cloning_view::clone_screen(s)
    } else if s.init.open {
        crate::initialize_view::init_screen(s)
    } else if s.accounts.open {
        crate::accounts_view::accounts_screen(s)
    } else if s.profiles_open {
        crate::profiles_view::profiles_screen(s)
    } else if s.history_open {
        crate::history_view::history_screen(s)
    } else if s.ignored.open {
        crate::ignored_view::ignored_screen(s)
    } else if s.branches.open {
        crate::branches_view::branches_screen(s)
    } else if s.tags.open {
        crate::tags_view::tags_screen(s)
    } else if s.remotes.open {
        crate::remotes_view::remotes_screen(s)
    } else if s.conflicts.open {
        crate::conflicts_view::conflicts_screen(s)
    } else if s.stashes.open {
        crate::stashes_view::stashes_screen(s)
    } else if s.reflog.open {
        crate::reflogs_view::reflog_screen(s)
    } else if s.file_history.open {
        crate::file_history_view::file_history_screen(s)
    } else if s.file_edit.open {
        crate::file_edit_view::file_edit_screen(s)
    } else if s.terminal.open {
        crate::terminal_view::terminal_screen(s)
    } else if s.undo_screen.open {
        crate::undos_view::undo_screen(s)
    } else if s.worktrees.open {
        crate::worktrees_view::worktrees_screen(s)
    } else if s.hooks.open {
        crate::hook_screen_view::hooks_screen(s)
    } else if s.gitflow.open {
        crate::gitflow_screen_view::gitflow_screen(s)
    } else if s.pulls.open {
        crate::pull_screen_view::pulls_screen(s)
    } else if s.submodules.open {
        crate::submodule_screen_view::submodules_screen(s)
    } else if s.lfs.open {
        crate::lfs_screen_view::lfs_screen(s)
    } else {
        body
    };
    let body = if s.commands.open {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(crate::palette_view::palette_bar(s))
            .child(body)
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
        _ => hints(s),
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

fn hints(s: &State) -> String {
    format!(
        "{} filtrer · {} et {} : fichier suivant ou précédent · {} et {} : changer de vue · {} : unifié ou séparé · ↑ ↓ ← → PgUp PgDn : lire · {} : actualiser · {} : indexer · {} : désindexer · {} : sélectionner · {} : appliquer à la sélection · clic sur un numéro de ligne : sélectionner (Maj pour étendre) · {} : ouvrir un dépôt · {} : palette de commandes",
        s.shortcut_for("filter"),
        s.shortcut_for("next-file"),
        s.shortcut_for("prev-file"),
        s.shortcut_for("view-unstaged"),
        s.shortcut_for("view-staged"),
        s.shortcut_for("toggle-mode"),
        s.shortcut_for("refresh-reading"),
        s.shortcut_for("stage"),
        s.shortcut_for("unstage"),
        s.shortcut_for("mark"),
        s.shortcut_for("apply-stage"),
        s.shortcut_for("open-repository"),
        s.shortcut_for("palette"),
    )
}

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
            .child(
                el().key("branch-name")
                    .role("button")
                    .label("Gérer les branches")
                    .focusable(true)
                    .focus_ring(2.0, 2.0, p.accent)
                    .hover(p.hover)
                    .radius(3.0)
                    .on_press(|s: &mut State| {
                        if s.branches.open {
                            s.close_branches();
                        } else {
                            s.open_branches();
                        }
                    })
                    .child(text(branch).font(theme::SMALL).color(p.accent).truncate()),
            );
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
        .px(if s.window_width < 900.0 { 12.0 } else { 20.0 })
        .gap(if s.window_width < 900.0 { 8.0 } else { 20.0 })
        .center_y()
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            row()
                .w(if s.window_width < 900.0 { 124.0 } else { 172.0 })
                .child(text("git").font(theme::WORDMARK).color(p.text))
                .child(text("poulpe").font(theme::WORDMARK_BOLD).color(p.text)),
        )
        .child(context)
        .child(open_button(&p, false))
        .child(history_button(s))
        .child(ignored_button(s))
        .child(profile_button(s))
        .child(refresh_button(s))
        .child(theme_toggle)
}

fn ignored_button(s: &State) -> El {
    let p = s.palette;
    el().key("ignored")
        .role("button")
        .label("Fichiers ignorés")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| {
            if s.ignored.open {
                s.close_ignored();
            } else {
                s.open_ignored(None);
            }
        })
        .child(text("Ignorés".to_owned()).font(theme::BODY).color(p.muted))
}

fn history_button(s: &State) -> El {
    let p = s.palette;
    el().key("history")
        .role("button")
        .label("Historique")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| {
            if s.history_open {
                s.close_history();
            } else {
                s.open_history();
            }
        })
        .child(
            text("Historique".to_owned())
                .font(theme::BODY)
                .color(p.muted),
        )
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

fn init_button(p: &Palette, here: Option<std::path::PathBuf>) -> El {
    let (key, caption) = if here.is_some() {
        ("init-here", "Initialiser ce dossier")
    } else {
        ("init", "Initialiser un dossier…")
    };
    el().key(key)
        .role("button")
        .label(caption)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .hover(p.hover)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| match &here {
            Some(path) => s.begin_init(path.clone()),
            None => crate::keys::init_dialog(tx),
        })
        .child(text(caption).font(theme::BODY).color(p.text))
}

fn clone_button(p: &Palette) -> El {
    el().key("clone")
        .role("button")
        .label("Cloner un dépôt")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .hover(p.hover)
        .on_press(|s: &mut State| s.begin_clone())
        .child(text("Cloner un dépôt…").font(theme::BODY).color(p.text))
}

fn message_screen(
    p: &Palette,
    title: &str,
    detail: &str,
    with_open: bool,
    here: Option<std::path::PathBuf>,
) -> El {
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
        let mut first = row().gap(8.0).child(open_button(p, here.is_none()));
        first = first.child(clone_button(p));
        let mut second = row().gap(8.0);
        if here.is_some() {
            second = second.child(init_button(p, here.clone()));
        }
        second = second.child(init_button(p, None));
        card = card.child(col().gap(8.0).child(first).child(second));
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

fn operation_banner(s: &State) -> Option<El> {
    let p = s.palette;
    let kind = s.operation_kind?;
    let conflicts = s
        .repo
        .as_ref()
        .map_or(0, |r| r.snapshot.conflicts().count());
    let message = if conflicts > 0 {
        format!(
            "{} en cours · {conflicts} fichier(s) en conflit",
            capitalized(kind.caption())
        )
    } else {
        format!("{} en cours", capitalized(kind.caption()))
    };
    Some(
        row()
            .key("operation-banner")
            .w_full()
            .center_y()
            .gap(12.0)
            .px(20.0)
            .py(8.0)
            .fill(p.panel)
            .border(1.0, p.line)
            .child(
                text(message)
                    .font(theme::BODY_STRONG)
                    .color(p.red)
                    .grow(1.0)
                    .truncate(),
            )
            .child(
                el().key("conflicts-open")
                    .role("button")
                    .label("Résoudre")
                    .focusable(true)
                    .focus_ring(2.0, 2.0, p.accent)
                    .h(30.0)
                    .px(12.0)
                    .center()
                    .radius(3.0)
                    .fill(p.accent)
                    .hover(p.hover)
                    .on_press(|s: &mut State| s.open_conflicts())
                    .child(text("Résoudre").font(theme::BODY_STRONG).color(p.on_accent)),
            ),
    )
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn workspace(s: &State) -> El {
    let body = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(left_pane(s))
        .child(divider(s))
        .child(right_pane(s));
    match operation_banner(s) {
        Some(banner) => col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(banner)
            .child(body),
        None => body,
    }
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
        ViewKind::Commit => "Changements apportés par ce commit.",
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
                .child(if s.view == ViewKind::Commit {
                    commit_card(s)
                } else {
                    row()
                        .w_full()
                        .gap(8.0)
                        .role("tablist")
                        .focus_group()
                        .child(tab(s, ViewKind::Unstaged, "Non indexés"))
                        .child(tab(s, ViewKind::Staged, "Indexés"))
                })
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
        .child(if s.view == ViewKind::Commit {
            el().h(0.0)
        } else {
            commit_box(s)
        })
}

fn commit_card(s: &State) -> El {
    let p = s.palette;
    let Some(inspected) = &s.inspected else {
        return el().h(0.0);
    };
    let detail = &inspected.detail;
    let short: String = detail.commit.oid.chars().take(8).collect();
    let mut card = col()
        .w_full()
        .gap(6.0)
        .child(
            text(detail.commit.subject.clone())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!(
                "{short} · {} <{}> · {} UTC",
                detail.commit.author,
                detail.author_email,
                crate::history_state::format_date(detail.commit.timestamp)
            ))
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    let body: Vec<&str> = detail.message.lines().skip(1).collect();
    if !body.is_empty() {
        card = card.child(
            text(body.join("\n").trim().to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap()
                .max_lines(6),
        );
    }
    let parents = &detail.commit.parents;
    card = card.child(match parents.len() {
        0 => text("Premier commit : comparé à un arbre vide.".to_owned())
            .font(theme::SMALL)
            .color(p.accent)
            .wrap(),
        1 => text(format!(
            "Comparé au parent {}.",
            parents[0].chars().take(8).collect::<String>()
        ))
        .font(theme::SMALL)
        .color(p.muted)
        .wrap(),
        _ => text("Commit de fusion : choisis le parent de comparaison.".to_owned())
            .font(theme::SMALL)
            .color(p.accent)
            .wrap(),
    });
    if parents.len() > 1 {
        let mut choices = row().w_full().gap(6.0);
        for (index, parent) in parents.iter().enumerate() {
            let selected = inspected.base_index == index;
            let caption = format!(
                "Parent {} · {}",
                index + 1,
                parent.chars().take(8).collect::<String>()
            );
            choices = choices.child(
                stage_button(
                    s,
                    format!("parent-{index}"),
                    caption.clone(),
                    caption,
                    selected,
                )
                .on_press(move |s: &mut State| s.choose_parent(index)),
            );
        }
        card = card.child(choices);
    }
    card.child(
        stage_button(
            s,
            "commit-back".to_owned(),
            "Quitter l'inspection du commit".to_owned(),
            "← Quitter l'inspection".to_owned(),
            false,
        )
        .on_press(|s: &mut State| s.leave_commit()),
    )
}

fn commit_box(s: &State) -> El {
    let p = s.palette;
    let staged = s.staged_for_commit().len();
    let mut lines = col().w_full().gap(2.0);
    if let Some(shown) = &s.identity_shown {
        let origin = shown
            .profile
            .as_ref()
            .map_or("configuration Git".to_owned(), |label| {
                format!("profil {label}")
            });
        match &shown.result {
            Ok(identity) => {
                lines = lines.child(
                    text(format!(
                        "Auteur : {} <{}> ({origin})",
                        identity.name, identity.email
                    ))
                    .font(theme::SMALL)
                    .color(p.muted)
                    .truncate(),
                );
            }
            Err(_) => {
                lines = lines.child(
                    text("Identité inconnue : choisis un profil avec le bouton Profil.".to_owned())
                        .font(theme::SMALL)
                        .color(p.red)
                        .wrap(),
                );
            }
        }
    }
    lines = lines.child(signing_summary(s));
    let input = text_input(&s.commit_message, theme::BODY)
        .key("commit-message")
        .label("Message du commit")
        .w_full()
        .h(72.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(
            "Message du commit (Ctrl+Entrée pour valider)".to_owned(),
            p.muted,
        )
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.commit_message)
        .on_input_key(|s: &mut State, event, _| {
            if event.mods.cmd && event.key == Key::Enter {
                s.commit();
                return true;
            }
            false
        });
    let caption = if s.amend_mode {
        match staged {
            0 => "Amender le commit".to_owned(),
            1 => "Amender le commit (+1 fichier)".to_owned(),
            n => format!("Amender le commit (+{n} fichiers)"),
        }
    } else {
        match staged {
            0 => "Créer le commit".to_owned(),
            1 => "Créer le commit (1 fichier)".to_owned(),
            n => format!("Créer le commit ({n} fichiers)"),
        }
    };
    let mut column = col()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(amend_toggle(s))
        .child(input)
        .child(lines);
    if let Some(warning) = amend_warning(s) {
        column = column.child(warning);
    }
    if let Some(report) = &s.commit_report {
        column = column.child(
            text(report.clone())
                .font(theme::SMALL)
                .color(p.red)
                .wrap()
                .max_lines(8),
        );
    }
    column.child(
        stage_button(
            s,
            "commit".to_owned(),
            "Créer le commit".to_owned(),
            caption,
            true,
        )
        .w_full()
        .h(32.0)
        .on_press(|s: &mut State| s.commit()),
    )
}

fn signing_summary(s: &State) -> El {
    let p = s.palette;
    let mode = match s.active_profile().map(|profile| &profile.signing) {
        Some(gitpoulpe_core::SigningPreference::Ssh { public_key }) => format!(
            "SSH, clé {}",
            public_key
                .file_name()
                .map(|n| display_label(Path::new(n)))
                .unwrap_or_default()
        ),
        Some(gitpoulpe_core::SigningPreference::Gpg { key_id }) => {
            let short: String = key_id
                .chars()
                .rev()
                .take(16)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("GPG, clé …{short}")
        }
        _ => {
            return text("Signature : selon la configuration Git".to_owned())
                .font(theme::SMALL)
                .color(p.muted);
        }
    };
    let (status, ink) = match s.identity_shown.as_ref().map(|shown| &shown.signing) {
        Some(gitpoulpe_git::SigningCheck::Ready) => ("utilisable", p.muted),
        Some(gitpoulpe_git::SigningCheck::Problem(_)) => ("impossible, voir Profil", p.red),
        _ => ("vérification en cours", p.muted),
    };
    text(format!("Signature : {mode} — {status}"))
        .font(theme::SMALL)
        .color(ink)
        .wrap()
}

fn amend_toggle(s: &State) -> El {
    let p = s.palette;
    let label = if s.amend_mode {
        "☑ Amender le dernier commit"
    } else {
        "☐ Amender le dernier commit"
    };
    el().key("amend-toggle")
        .role("checkbox")
        .label("Amender le dernier commit")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(26.0)
        .px(2.0)
        .center_y()
        .hover(p.hover)
        .on_press(|s: &mut State| {
            let on = !s.amend_mode;
            s.set_amend_mode(on);
        })
        .child(text(label.to_owned()).font(theme::SMALL).color(p.text))
}

fn amend_warning(s: &State) -> Option<El> {
    if !s.amend_mode {
        return None;
    }
    let p = s.palette;
    let info = s.amend_info.as_ref()?;
    let short: String = info.head.chars().take(8).collect();
    let subject = info.message.lines().next().unwrap_or_default().to_owned();
    let mut card = col().w_full().gap(6.0).child(
        text(format!("Commit amendé : {short} « {subject} »"))
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
    );
    if !info.published_on.is_empty() {
        card = card
            .child(
                text(format!(
                    "Ce commit est déjà publié sur {}. L'amender réécrit l'historique : le prochain envoi devra être forcé et gênera les personnes qui l'ont déjà récupéré. L'ancien commit {short} reste récupérable.",
                    info.published_on.join(", ")
                ))
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
            );
        if s.amend_confirming {
            card = card.child(
                text("Clique sur « Amender le commit » pour confirmer.".to_owned())
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
    }
    Some(card)
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
    let code = if view == ViewKind::Unstaged {
        entry.worktree
    } else {
        entry.index
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
                        .child(edit_action(s, index, entry, &path))
                        .child(history_action(s, index, entry, &path))
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

fn edit_action(s: &State, index: usize, entry: &ChangeEntry, label: &str) -> El {
    if s.view != ViewKind::Unstaged || entry.is_conflict() || !entry.is_unstaged() {
        return el().w(0.0);
    }
    let Some(target) = entry.path.to_str().map(str::to_owned) else {
        return el().w(0.0);
    };
    stage_button(
        s,
        format!("ed{index}"),
        format!("Éditer {label}"),
        "Éditer".to_owned(),
        false,
    )
    .on_press(move |s: &mut State| s.open_file_edit(target.clone()))
}

fn history_action(s: &State, index: usize, entry: &ChangeEntry, label: &str) -> El {
    let Some(target) = entry.path.to_str().map(str::to_owned) else {
        return el().w(0.0);
    };
    stage_button(
        s,
        format!("fh{index}"),
        format!("Historique de {label}"),
        "Historique".to_owned(),
        false,
    )
    .on_press(move |s: &mut State| s.open_file_history(target.clone()))
}

fn header_action(s: &State, index: usize, entry: &ChangeEntry, label: &str) -> El {
    if s.view == ViewKind::Commit {
        return el().w(0.0);
    }
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
    let ignore_button =
        (staging && entry.kind == gitpoulpe_core::EntryKind::Untracked).then(|| {
            let target = entry.path.clone();
            stage_button(
                s,
                format!("ig{index}"),
                format!("Ignorer {label}"),
                "Ignorer…".to_owned(),
                false,
            )
            .on_press(move |s: &mut State| s.open_ignored(Some(target.clone())))
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
        .child(ignore_button.unwrap_or_else(|| el().w(0.0)))
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
        ViewKind::Commit => false,
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
