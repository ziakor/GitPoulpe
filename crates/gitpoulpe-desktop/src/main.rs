use std::borrow::Cow;

use rune::tok::{self, type_};
use rune::{AppSpec, Cx, El, GrowthSpec, ScrollSpec, TextEdit, col, el, row, text, text_input};

const ROW_COUNT: usize = 100_000;
const ROW_HEIGHT: f32 = 24.0;

struct State {
    dark: bool,
    field: TextEdit,
}

impl Default for State {
    fn default() -> Self {
        Self {
            dark: true,
            field: TextEdit::new(""),
        }
    }
}

fn row_key(_: &State, i: usize) -> Cow<'_, str> {
    Cow::Owned(format!("row-{i}"))
}

fn build_row(s: &State, i: usize) -> El {
    let t = tok::theme(s.dark);
    row().w_full().px(12.0).h(ROW_HEIGHT).center_y().child(
        text(format!("{i:06}  qualification row {i}"))
            .font(type_::BODY)
            .color(t.text)
            .selectable(true),
    )
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(s.dark);
    let field = text_input(&s.field, type_::BODY)
        .key("field")
        .label("Qualification field")
        .single_line()
        .grow(1.0)
        .h(32.0)
        .px(10.0)
        .radius(6.0)
        .fill(t.card)
        .color(t.text)
        .placeholder("Type here", t.text_secondary)
        .on_edit(|s: &mut State| &mut s.field);
    let toggle = el()
        .key("theme-toggle")
        .role("button")
        .label("Toggle theme")
        .focusable(true)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(6.0)
        .fill(t.card)
        .on_press(|s: &mut State| s.dark = !s.dark)
        .child(text(if s.dark { "Dark" } else { "Light" }).color(t.text));
    let list = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(ROW_COUNT, ROW_HEIGHT, row_key, build_row);
    col()
        .w_full()
        .h_full()
        .fill(t.bg)
        .padding(12.0)
        .gap(12.0)
        .child(row().w_full().gap(12.0).child(field).child(toggle))
        .child(
            col()
                .key("viewport")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

fn main() {
    rune::run(AppSpec::<State> {
        scenes: &[("light", |s| s.dark = false)],
        ..AppSpec::new("GitPoulpe", (960.0, 640.0), State::default, view)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Key, KeyEvent, Mods, Runtime};

    fn runtime() -> Runtime<State> {
        let env = Env {
            system_dark: true,
            scale: 1.0,
        };
        let mut rt = Runtime::new(State::default(), view, (960.0, 640.0), env);
        rt.frame(0.0);
        rt
    }

    #[test]
    fn mounted_rows_stay_bounded_for_one_hundred_thousand_rows() {
        let rt = runtime();
        let mounted = rt
            .elements()
            .into_iter()
            .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("row-")))
            .count();
        assert!(mounted > 0 && mounted < 500, "mounted rows: {mounted}");
    }

    #[test]
    fn pressing_the_toggle_switches_the_theme() {
        let mut rt = runtime();
        let (x, y) = rt.find("theme-toggle").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.dark);
    }

    fn type_text(rt: &mut Runtime<State>, text: &str) {
        rt.key(KeyEvent {
            key: Key::Text(text.to_owned()),
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }

    fn command(rt: &mut Runtime<State>, c: char) {
        let mods = Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        };
        rt.key(KeyEvent {
            key: Key::Command(c),
            mods,
        });
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn typing_and_copying_work_in_both_themes() {
        for dark in [true, false] {
            let mut rt = runtime();
            let clipboard = rune::clipboard::memory();
            rt.use_clipboard(&clipboard);
            if !dark {
                let (x, y) = rt.find("theme-toggle").unwrap();
                rt.pointer_down(x, y);
                rt.pointer_up(x, y);
                rt.frame(1.0 / 60.0);
            }
            assert_eq!(rt.state.dark, dark);
            let (x, y) = rt.find("field").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(1.0 / 60.0);
            type_text(&mut rt, "gitpoulpe");
            assert_eq!(rt.state.field.text, "gitpoulpe");
            command(&mut rt, 'a');
            command(&mut rt, 'c');
            assert_eq!(
                clipboard.text().as_deref().map(String::as_str),
                Some("gitpoulpe")
            );
        }
    }
}
