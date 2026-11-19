use rune::{El, ScrollSpec, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;

const MAX_ROWS: usize = 8;

pub fn palette_bar(s: &State) -> El {
    let p = s.palette;
    let results = s.palette_results();
    let selected = s.palette_selected_id();
    let input = text_input(&s.commands.query, theme::BODY)
        .key("pal-input")
        .label("Rechercher une commande")
        .single_line()
        .w_full()
        .h(38.0)
        .px(12.0)
        .py(9.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder("Rechercher une commande…".to_owned(), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.commands.query)
        .on_input_key(|s: &mut State, event, tx| s.palette_key(&event.key, tx));
    let mut list = col()
        .key("pal-list")
        .role("listbox")
        .w_full()
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if results.is_empty() {
        list = list.child(
            text(
                "Aucune commande disponible pour cette recherche dans le contexte actuel."
                    .to_owned(),
            )
            .key("pal-empty")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    for (index, command) in results.iter().enumerate() {
        let id = command.id;
        let active = selected == Some(id);
        list = list.child(
            el().key(format!("pal-row{index}"))
                .role("option")
                .label(command.label.to_owned())
                .w_full()
                .h(34.0)
                .px(12.0)
                .radius(3.0)
                .fill(if active { p.selected } else { p.bg })
                .hover(p.hover)
                .on_press_tx(move |s: &mut State, tx| s.palette_run(Some(id), tx))
                .child(
                    row().w_full().h(34.0).center_y().child(
                        text(command.label.to_owned())
                            .font(theme::BODY)
                            .color(p.text)
                            .truncate(),
                    ),
                ),
        );
    }
    let rows = results.len().clamp(1, MAX_ROWS) as f32;
    col()
        .key("pal-bar")
        .w_full()
        .px(20.0)
        .py(10.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(input)
        .child(list.h(rows * 36.0))
}
