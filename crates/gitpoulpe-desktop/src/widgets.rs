use rune::{El, Key, TextEdit, el, text, text_input};

use crate::state::State;
use crate::theme;

pub fn action(s: &State, key: &str, caption: &str, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key.to_owned())
        .role("button")
        .label(caption.to_owned())
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
        .child(text(caption.to_owned()).font(theme::BODY).color(ink).wrap())
}

pub fn field(s: &State, key: &str, label: &str, edit: &TextEdit, placeholder: &str) -> El {
    let p = s.palette;
    text_input(edit, theme::BODY)
        .key(key.to_owned())
        .label(label.to_owned())
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(placeholder.to_owned(), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
        .on_input_key(|_: &mut State, event, tx| {
            if event.key == Key::Escape {
                tx.blur();
                return true;
            }
            false
        })
}

pub fn toggle(s: &State, key: &str, caption: &str, on: bool) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("checkbox")
        .label(caption.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .min_h(28.0)
        .px(8.0)
        .py(4.0)
        .radius(3.0)
        .hover(p.hover)
        .child(
            text(format!("{} {caption}", if on { "[x]" } else { "[ ]" }))
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        )
}
