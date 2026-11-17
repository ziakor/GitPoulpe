use rune::{FileDialogOptions, Key, KeyEvent, SelectedText, Tx};

use crate::state::{DiffMode, SelectionAnchor, State, ViewKind};
use crate::view::row_key;

const LINES_PER_KEY: usize = 3;
const SIDEWAYS_STEP: f32 = 96.0;

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

pub fn on_key(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.mods.cmd && event.is_command('c') {
        return copy_selection(s, tx);
    }
    s.notice = None;
    if event.mods.cmd {
        if event.is_command('o') {
            open_dialog(tx);
            return true;
        }
        if event.is_command('r') {
            s.refresh();
            return true;
        }
        return false;
    }
    if event.mods.alt || event.mods.ctrl {
        return false;
    }
    let reading = matches!(s.screen, crate::state::Screen::Ready);
    let free = tx.focused().is_none() || tx.focused() == Some("viewport");
    match &event.key {
        Key::Text(text) if reading && free => match text.as_str() {
            "/" => tx.focus("filter", true),
            "m" => s.toggle_mode(tx),
            "r" => s.refresh(),
            "s" => {
                if s.line_pick.is_some() {
                    s.stage_picked_lines();
                } else {
                    s.stage_current();
                }
            }
            "S" => s.stage_marked(),
            "x" => {
                if let Some(position) = s.current {
                    s.toggle_mark(position);
                }
            }
            "j" => reveal_file(s, tx, 1),
            "k" => reveal_file(s, tx, -1),
            "1" => s.set_view(ViewKind::Unstaged),
            "2" => s.set_view(ViewKind::Staged),
            _ => return false,
        },
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
