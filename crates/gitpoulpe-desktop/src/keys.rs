use rune::{FileDialogOptions, Key, KeyEvent, Tx};

use crate::state::{State, ViewKind};
use crate::view::row_key;

const LINES_PER_KEY: usize = 3;

pub fn open_dialog(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Ouvrir un dépôt Git".into(),
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut State, result, _| {
        if let Ok(Some(path)) = result {
            s.open_path(path);
        }
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

pub fn on_key(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.mods.cmd {
        if event.is_command('o') {
            open_dialog(tx);
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
            "j" => reveal_file(s, tx, 1),
            "k" => reveal_file(s, tx, -1),
            "1" => s.set_view(ViewKind::Unstaged),
            "2" => s.set_view(ViewKind::Staged),
            _ => return false,
        },
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
