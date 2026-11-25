use rune::{Color, El, Key, LineCap, Shape, TextEdit, canvas, col, el, row, text, text_input};

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

pub fn check_mark(s: &State, on: bool) -> El {
    let p = s.palette;
    el().w(16.0)
        .h(16.0)
        .radius(3.0)
        .center()
        .fill(if on { p.accent } else { p.raised })
        .border(1.5, if on { p.accent } else { p.edge })
        .child(
            text(if on { "✓" } else { "" })
                .font(theme::TINY)
                .color(p.on_accent),
        )
}

pub fn check(s: &State, key: &str, label: &str, on: bool) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("checkbox")
        .label(label.to_owned())
        .checked(on)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .w(28.0)
        .h(28.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .child(check_mark(s, on))
}

pub fn toggle(s: &State, key: &str, caption: &str, on: bool) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("checkbox")
        .label(caption.to_owned())
        .checked(on)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .min_h(28.0)
        .px(8.0)
        .py(4.0)
        .radius(3.0)
        .hover(p.hover)
        .child(
            row().gap(8.0).center_y().child(check_mark(s, on)).child(
                text(caption.to_owned())
                    .font(theme::SMALL)
                    .color(p.text)
                    .shrink()
                    .wrap(),
            ),
        )
}

pub fn area(s: &State, key: &str, label: &str, edit: &TextEdit, placeholder: &str) -> El {
    let p = s.palette;
    text_input(edit, theme::BODY)
        .key(key.to_owned())
        .label(label.to_owned())
        .w_full()
        .h(76.0)
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

pub fn text_tab(s: &State, key: &str, caption: String, active: bool) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("tab")
        .label(caption.clone())
        .focus_selected(active)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .py(9.0)
        .border_sides(
            [0.0, 0.0, 2.0, 0.0],
            if active {
                p.accent
            } else {
                rune::Color::TRANSPARENT
            },
        )
        .hover(p.hover)
        .child(
            text(caption)
                .font(if active {
                    theme::BODY_STRONG
                } else {
                    theme::BODY_MEDIUM
                })
                .color(if active { p.text } else { p.muted }),
        )
}

pub fn choice(s: &State, key: &str, caption: &str, hint: Option<&str>, chosen: bool) -> El {
    let p = s.palette;
    let mark = el().w(12.0).h(12.0).mt(3.0).radius(6.0).border(
        if chosen { 4.0 } else { 1.0 },
        if chosen { p.accent } else { p.muted },
    );
    let mut words = col().items_start().gap(1.0).grow(1.0).min_w(0.0).child(
        text(caption.to_owned())
            .font(theme::BODY)
            .color(p.text)
            .wrap(),
    );
    if let Some(hint) = hint {
        words = words.child(
            text(hint.to_owned())
                .font(theme::TINY)
                .color(p.muted)
                .wrap(),
        );
    }
    row()
        .key(key.to_owned())
        .role("radio")
        .label(caption.to_owned())
        .focus_selected(chosen)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .w_full()
        .items_start()
        .gap(10.0)
        .px(8.0)
        .py(6.0)
        .radius(3.0)
        .hover(p.hover)
        .child(mark)
        .child(words)
}

pub fn quiet(s: &State, key: &str, caption: &str) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("button")
        .label(caption.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .px(4.0)
        .py(2.0)
        .radius(3.0)
        .hover(p.hover)
        .child(text(caption.to_owned()).font(theme::SMALL).color(p.accent))
}

pub fn panel_icon(color: Color, collapse: bool) -> El {
    let (back, tip) = if collapse { (11.5, 9.0) } else { (9.0, 11.5) };
    let shape = Shape::new()
        .stroke_with(
            vec![[1.75, 2.75], [14.25, 2.75], [14.25, 13.25], [1.75, 13.25]],
            true,
            1.5,
            LineCap::Butt,
            color,
        )
        .line([6.0, 2.75], [6.0, 13.25], 1.5, LineCap::Butt, color)
        .stroke_with(
            vec![[back, 5.5], [tip, 8.0], [back, 10.5]],
            false,
            1.5,
            LineCap::Round,
            color,
        );
    canvas(shape).w(16.0).h(16.0)
}

pub fn icon_button(s: &State, key: &str, label: &str, icon: El) -> El {
    let p = s.palette;
    el().key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .help(label.to_owned())
        .focusable(true)
        .focus_ring(2.0, 0.0, p.accent)
        .w(28.0)
        .h(28.0)
        .center()
        .radius(4.0)
        .hover(p.hover)
        .child(icon)
}

pub fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

pub fn screen_head(s: &State, key: &str, title: &str, context: String, actions: Vec<El>) -> El {
    let p = s.palette;
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(
        text(title.to_owned())
            .key(format!("{key}-title"))
            .font(theme::HEADING)
            .color(p.text)
            .w_full()
            .truncate(),
    );
    if !context.is_empty() {
        words = words.child(
            text(context)
                .key(format!("{key}-context"))
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .truncate(),
        );
    }
    let mut head = row()
        .key(key.to_owned())
        .w_full()
        .center_y()
        .px(20.0)
        .py(12.0)
        .gap(8.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .child(words);
    for action in actions {
        head = head.child(action);
    }
    head
}
