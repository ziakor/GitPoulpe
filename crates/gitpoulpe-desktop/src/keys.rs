use rune::{FileDialogOptions, Key, KeyEvent, SelectedText, Tx};

use crate::state::{DiffMode, SelectionAnchor, State, ViewKind};
use crate::view::row_key;

const LINES_PER_KEY: usize = 3;
const SIDEWAYS_STEP: f32 = 96.0;

const TEXT_INPUTS: [&str; 47] = [
    "ac-server",
    "ac-token",
    "br-filter",
    "pr-title",
    "pr-body",
    "pr-base",
    "prc-body",
    "prv-body",
    "mrg-message",
    "gf-name",
    "gf-main",
    "gf-develop",
    "gf-feature",
    "gf-release",
    "gf-hotfix",
    "gf-tag",
    "wt-path",
    "wt-branch",
    "wt-reason",
    "rl-name",
    "stash-message",
    "cf-result",
    "clone-url",
    "clone-name",
    "rm-new-name",
    "rm-new-url",
    "rm-url",
    "rm-push-url",
    "rm-rename",
    "init-branch",
    "filter",
    "commit-message",
    "history-search",
    "pf-label",
    "pf-name",
    "pf-email",
    "pf-key",
    "pal-input",
    "sw-input",
    "sc-search",
    "ws-new-name",
    "pt-revision",
    "pt-dest",
    "lk-input",
    "ws-name",
    "fv-search",
    "is-query",
];

pub fn open_dialog(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Ouvrir un dépôt Git".into(),
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => s.open_path(path),
        Ok(None) => {}
        Err(error) => s.notice = Some(format!("Sélecteur de dossier indisponible : {error}")),
    });
}

pub fn init_dialog(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Choisir le dossier à initialiser".into(),
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => s.begin_init(path),
        Ok(None) => {}
        Err(error) => s.notice = Some(format!("Sélecteur de dossier indisponible : {error}")),
    });
}

pub fn clone_parent_dialog(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Dossier de destination du clone".into(),
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => s.set_clone_parent(path),
        Ok(None) => {}
        Err(error) => s.notice = Some(format!("Sélecteur de dossier indisponible : {error}")),
    });
}

pub fn align_file(tx: &mut Tx, index: usize) {
    tx.scroll_key_to(format!("h{index}"), 0.0);
}

pub fn reveal_file(s: &mut State, tx: &mut Tx, delta: isize) {
    if let Some(index) = s.step_file(delta) {
        align_file(tx, index);
    }
}

fn scroll_lines(s: &mut State, tx: &mut Tx, down: bool) {
    if !down {
        s.stick_end = false;
    }
    let total = s.layout.total as usize;
    if total == 0 {
        return;
    }
    let target = if down {
        (s.visible.end + LINES_PER_KEY - 1).min(total - 1)
    } else {
        s.visible.start.saturating_sub(LINES_PER_KEY)
    };
    tx.scroll_into_view_in("rows", row_key(s, target));
}

fn anchor_of(
    text: Option<&SelectedText>,
    at_start: bool,
) -> Option<(SelectionAnchor, Option<bool>)> {
    let text = text?;
    let rest = text.key()?.strip_prefix('t')?;
    let mut parts = rest.split(':');
    let index = parts.next()?.parse().ok()?;
    let row = parts.next()?.parse().ok()?;
    let left = parts.next().map(|side| side == "L");
    let range = text.range();
    let offset = if at_start { range.start } else { range.end };
    Some((SelectionAnchor { index, row, offset }, left))
}

fn copy_selection(s: &mut State, tx: &Tx) -> bool {
    let Some(selection) = tx.text_selection() else {
        return false;
    };
    let texts = selection.texts();
    if texts.len() < 2 {
        return false;
    }
    let (Some((first, left)), Some((last, _))) = (
        anchor_of(texts.first(), true),
        anchor_of(texts.last(), false),
    ) else {
        return false;
    };
    s.start_copy(first, last, left)
}

fn run_action(s: &mut State, tx: &mut Tx, id: &str) -> bool {
    match id {
        "filter" => tx.focus("filter", true),
        "toggle-mode" => s.toggle_mode(tx),
        "refresh-reading" | "refresh" => s.refresh(),
        "stage" => {
            if s.line_pick.is_some() {
                s.write_picked_lines();
            } else {
                s.stage_current();
            }
        }
        "unstage" => {
            if s.line_pick.is_some() {
                s.write_picked_lines();
            } else {
                s.unstage_current();
            }
        }
        "apply-stage" | "apply-unstage" => s.write_marked(),
        "mark" => {
            if let Some(position) = s.current {
                s.toggle_mark(position);
            }
        }
        "next-file" => reveal_file(s, tx, 1),
        "prev-file" => reveal_file(s, tx, -1),
        "view-unstaged" => s.set_view(ViewKind::Unstaged),
        "view-staged" => s.set_view(ViewKind::Staged),
        "open-repository" => open_dialog(tx),
        "palette" => {
            if s.commands.open {
                s.close_palette(tx, true);
            } else {
                s.open_palette(tx);
            }
        }
        _ => return false,
    }
    true
}

pub fn on_key(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if s.terminal.open && s.terminal.session.is_some() {
        return s.terminal_key(event);
    }
    if s.shortcuts.capturing.is_some() {
        return s.shortcut_capture(event);
    }
    if event.mods.cmd && event.is_command('c') {
        return copy_selection(s, tx);
    }
    s.notice = None;
    if s.links.open && event.key == Key::Escape {
        s.close_deep_link();
        return true;
    }
    if s.patches.open && event.key == Key::Escape {
        s.close_patch_import();
        return true;
    }
    if s.issues.open && event.key == Key::Escape {
        s.close_issues();
        return true;
    }
    if s.notices.open && event.key == Key::Escape {
        s.close_notices();
        return true;
    }
    if s.overview.open && event.key == Key::Escape {
        s.close_overview();
        return true;
    }
    if s.favorites.open && event.key == Key::Escape {
        s.close_favorites();
        return true;
    }
    if s.workspaces.open && event.key == Key::Escape {
        s.close_workspaces();
        return true;
    }
    if s.shortcuts.open && event.key == Key::Escape {
        s.close_shortcuts();
        return true;
    }
    if s.commands.open && s.palette_key(&event.key, tx) {
        return true;
    }
    if s.switcher.open && s.switcher_key(&event.key, tx) {
        return true;
    }
    if s.tools.open && event.key == Key::Escape {
        s.close_tools();
        return true;
    }
    if s.clone.open && event.key == Key::Escape {
        s.close_clone();
        return true;
    }
    if s.init.open && event.key == Key::Escape {
        s.cancel_init();
        return true;
    }
    if s.profiles_open && event.key == Key::Escape {
        s.close_profiles();
        return true;
    }
    if s.accounts.browser.open && event.key == Key::Escape {
        s.close_browser();
        return true;
    }
    if s.accounts.open && event.key == Key::Escape {
        s.close_accounts();
        return true;
    }
    if s.pulls.list.open && event.key == Key::Escape {
        s.close_pull_list();
        return true;
    }
    if s.pulls.open && event.key == Key::Escape {
        s.close_pulls();
        return true;
    }
    if s.gitflow.open && event.key == Key::Escape {
        s.close_gitflow();
        return true;
    }
    if s.submodules.open && event.key == Key::Escape {
        s.close_submodules();
        return true;
    }
    if s.lfs.open && event.key == Key::Escape {
        s.close_lfs();
        return true;
    }
    if s.hooks.open && event.key == Key::Escape {
        if s.hooks.confirm_shared.is_some() {
            s.hook_cancel_shared();
        } else {
            s.close_hooks();
        }
        return true;
    }
    if s.worktrees.open && event.key == Key::Escape {
        if s.worktrees.removal.is_some() {
            s.worktree_cancel_remove();
        } else {
            s.close_worktrees();
        }
        return true;
    }
    if s.undo_screen.open && event.key == Key::Escape {
        s.close_undo();
        return true;
    }
    if s.reflog.open && event.key == Key::Escape {
        s.close_reflog();
        return true;
    }
    if s.file_history.open && event.key == Key::Escape {
        s.close_file_history();
        return true;
    }
    if s.file_edit.open && event.key == Key::Escape {
        s.file_edit_close_request();
        return true;
    }
    if s.stashes.open && event.key == Key::Escape {
        if s.stashes.confirm_drop.is_some() {
            s.stash_cancel_drop();
        } else {
            s.close_stashes();
        }
        return true;
    }
    if s.conflicts.open && event.key == Key::Escape {
        if s.conflicts.confirm_abort || s.conflicts.markers_pending {
            s.conflict_cancel_abort();
        } else {
            s.close_conflicts();
        }
        return true;
    }
    if s.remotes.open && event.key == Key::Escape {
        if s.remotes.confirm_remove.is_some() {
            s.remote_cancel_remove();
        } else {
            s.close_remotes();
        }
        return true;
    }
    if s.tags.open && event.key == Key::Escape {
        if s.tags.confirm_delete.is_some() {
            s.tag_cancel_delete();
        } else {
            s.close_tags();
        }
        return true;
    }
    if s.branches.open && event.key == Key::Escape {
        if s.branches.confirm.is_some() {
            s.branch_cancel_delete();
        } else {
            s.close_branches();
        }
        return true;
    }
    if s.ignored.open && event.key == Key::Escape {
        s.close_ignored();
        return true;
    }
    if s.history_open && event.key == Key::Escape {
        s.close_history();
        return true;
    }
    if s.view == ViewKind::Commit && event.key == Key::Escape {
        s.leave_commit();
        return true;
    }
    if let Some(chord) = crate::shortcuts::chord_of(event)
        && let Some(id) = s
            .shortcuts
            .action_for(&chord, crate::shortcuts::Scope::Global)
    {
        return run_action(s, tx, id);
    }
    if event.mods.cmd {
        return false;
    }
    if event.mods.alt || event.mods.ctrl {
        return false;
    }
    let reading = matches!(s.screen, crate::state::Screen::Ready);
    let free = tx.focused().is_none() || tx.focused() == Some("viewport");
    let typing = tx.focused().is_some_and(|key| TEXT_INPUTS.contains(&key))
        || s.init.open
        || s.clone.open
        || s.profiles_open
        || s.ignored.open
        || s.branches.open
        || s.tags.open
        || s.remotes.open
        || s.conflicts.open
        || s.stashes.open
        || s.reflog.open
        || s.file_history.open
        || s.file_edit.open
        || s.terminal.open
        || s.commands.open
        || s.switcher.open
        || s.shortcuts.open
        || s.workspaces.open
        || s.favorites.open
        || s.overview.open
        || s.notices.open
        || s.issues.open
        || s.patches.open
        || s.links.open
        || s.undo_screen.open
        || s.worktrees.open
        || s.hooks.open
        || s.lfs.open
        || s.submodules.open
        || s.gitflow.open
        || s.pulls.open
        || s.accounts.open;
    match &event.key {
        Key::Text(text) if reading && !typing => {
            match s
                .shortcuts
                .action_for(text, crate::shortcuts::Scope::Reading)
            {
                Some(id) => return run_action(s, tx, id),
                None => return false,
            }
        }
        Key::Left | Key::Right if reading && free && s.mode == DiffMode::Unified => {
            let [offset, max] = s.scroll_x.get();
            let step = if event.key == Key::Right {
                SIDEWAYS_STEP
            } else {
                -SIDEWAYS_STEP
            };
            tx.scroll_to_x("viewport", (offset + step).clamp(0.0, max.max(0.0)));
        }
        Key::Escape if reading && free && s.line_pick.is_some() => s.clear_pick(),
        Key::Down if reading && free => scroll_lines(s, tx, true),
        Key::Up if reading && free => scroll_lines(s, tx, false),
        Key::PageDown if reading && tx.focused().is_none() => tx.scroll_page("viewport", 1),
        Key::PageUp if reading && tx.focused().is_none() => {
            s.stick_end = false;
            tx.scroll_page("viewport", -1);
        }
        Key::Home if reading && tx.focused().is_none() => {
            s.stick_end = false;
            tx.scroll_to_y("viewport", 0.0);
        }
        Key::End if reading && tx.focused().is_none() => {
            s.stick_to_end();
            tx.scroll_to_bottom_instant("viewport");
        }
        _ => return false,
    }
    true
}
