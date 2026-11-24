use rune::{El, ScrollSpec, col, el, row, text, text_input};

use crate::shortcuts::{ACTIONS, display};
use crate::state::State;
use crate::theme;
use crate::widgets::{action, screen_head};

pub fn shortcuts_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.shortcuts;
    let header = screen_head(
        s,
        "sc-head",
        "Raccourcis clavier",
        String::new(),
        vec![
            action(s, "sc-modify", "Modifier", true).on_press_tx(|s: &mut State, tx| {
                s.shortcut_begin_capture();
                tx.blur();
            }),
            action(s, "sc-reset", "Rétablir", false).on_press(|s: &mut State| s.shortcut_reset()),
            action(s, "sc-reset-all", "Tout rétablir", false)
                .on_press(|s: &mut State| s.shortcut_reset_all()),
            action(s, "sc-close", "Fermer", false).on_press(|s: &mut State| s.close_shortcuts()),
        ],
    );
    let search = text_input(&screen.query, theme::BODY)
        .key("sc-search")
        .label("Rechercher un raccourci")
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(
            "Rechercher une action ou une combinaison…".to_owned(),
            p.muted,
        )
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.shortcuts.query);
    let mut notes = col().w_full().gap(4.0);
    if let Some(id) = screen.capturing {
        let label = ACTIONS.iter().find(|a| a.id == id).map_or("", |a| a.label);
        notes = notes.child(
            text(format!(
                "Appuyez sur la nouvelle combinaison pour « {label} » (Échap pour annuler)."
            ))
            .key("sc-capturing")
            .font(theme::SMALL)
            .color(p.accent)
            .wrap(),
        );
    }
    if let Some(error) = &screen.error {
        notes = notes.child(
            text(error.clone())
                .key("sc-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &screen.notice {
        notes = notes.child(
            text(notice.clone())
                .key("sc-notice")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    let mut list = col()
        .key("sc-list")
        .role("listbox")
        .w_full()
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    for (index, entry) in s.shortcut_visible().iter().enumerate() {
        let id = entry.id;
        let chord = display(&screen.chord(entry));
        let custom = screen.custom.contains_key(id);
        let active = screen.selected == Some(id);
        list = list.child(
            el().key(format!("sc-row{index}"))
                .role("option")
                .label(format!("{} : {chord}", entry.label))
                .w_full()
                .h(36.0)
                .px(12.0)
                .radius(3.0)
                .fill(if active { p.selected } else { p.bg })
                .hover(p.hover)
                .on_press(move |s: &mut State| s.shortcut_select(id))
                .child(
                    row()
                        .w_full()
                        .h(36.0)
                        .center_y()
                        .gap(12.0)
                        .child(
                            text(entry.label.to_owned())
                                .font(theme::BODY)
                                .color(p.text)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child(
                            text(if custom {
                                format!("{chord} · personnalisé")
                            } else {
                                chord
                            })
                            .font(theme::BODY_STRONG)
                            .color(if custom {
                                p.accent
                            } else {
                                p.muted
                            }),
                        ),
                ),
        );
    }
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(20.0)
            .gap(8.0)
            .child(search)
            .child(notes)
            .child(list),
    )
}
