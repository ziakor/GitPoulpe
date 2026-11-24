use rune::{El, ScrollSpec, Tx, col, el, row, text, text_input};

use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 44.0;
const MAX_ROWS: usize = 6;

fn link(s: &State, key: &'static str, caption: String) -> El {
    let p = s.palette;
    el().key(key)
        .role("button")
        .label(caption.clone())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .px(8.0)
        .py(4.0)
        .radius(3.0)
        .hover(p.hover)
        .child(text(caption).font(theme::SMALL).color(p.accent))
}

pub fn switcher_bar(s: &State) -> El {
    let p = s.palette;
    let results = s.switcher_results();
    let selected = s.switcher_selected();
    let input = text_input(&s.switcher.query, theme::BODY)
        .key("sw-input")
        .label("Filtrer les dépôts connus")
        .single_line()
        .w_full()
        .h(38.0)
        .px(12.0)
        .py(9.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(
            "Filtrer les favoris et les dépôts des espaces de travail…".to_owned(),
            p.muted,
        )
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.switcher.query)
        .on_input_key(|s: &mut State, event, tx| s.switcher_key(&event.key, tx));
    let mut list = col()
        .key("sw-list")
        .role("listbox")
        .w_full()
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if results.is_empty() {
        let message = if s.switcher.candidates.is_empty() {
            "Aucun autre dépôt connu : ajoutez des favoris ou des espaces de travail, ou ouvrez un dépôt."
        } else {
            "Aucun dépôt ne correspond à ce filtre."
        };
        list = list.child(
            text(message.to_owned())
                .key("sw-empty")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    for (index, candidate) in results.iter().enumerate() {
        let path = candidate.path.clone();
        let active = selected.as_deref() == Some(candidate.path.as_str());
        list = list.child(
            row()
                .key(format!("sw-row{index}"))
                .role("option")
                .label(format!("{}, {}", candidate.name, candidate.path))
                .focus_selected(active)
                .w_full()
                .h(ROW_HEIGHT)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .radius(3.0)
                .fill(if active { p.selected } else { p.bg })
                .hover(p.hover)
                .on_press_tx(move |s: &mut State, tx: &mut Tx| s.switch_to(Some(path.clone()), tx))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(1.0)
                        .child(
                            text(candidate.name.clone())
                                .font(theme::BODY_MEDIUM)
                                .color(p.text)
                                .truncate(),
                        )
                        .child(
                            text(candidate.path.clone())
                                .font(theme::PATH_SMALL)
                                .color(p.muted)
                                .truncate_middle(),
                        ),
                )
                .child(
                    text(candidate.sources.join(" · "))
                        .font(theme::SMALL)
                        .color(p.muted)
                        .shrink()
                        .truncate(),
                ),
        );
    }
    let rows = results.len().clamp(1, MAX_ROWS) as f32;
    let mut actions = row()
        .w_full()
        .gap(4.0)
        .flex_wrap()
        .center_y()
        .child(
            link(s, "sw-open", "Ouvrir un dépôt…".to_owned()).on_press_tx(
                |s: &mut State, tx: &mut Tx| {
                    s.close_switcher(tx, false);
                    crate::keys::open_dialog(tx);
                },
            ),
        )
        .child(
            link(s, "sw-clone", "Cloner un dépôt".to_owned()).on_press_tx(
                |s: &mut State, tx: &mut Tx| {
                    s.close_switcher(tx, false);
                    s.close_screens();
                    s.begin_clone();
                },
            ),
        );
    if s.repo.is_some() && !s.current_is_favorite() {
        let name = s.repo.as_ref().map(|r| r.name.clone()).unwrap_or_default();
        actions = actions.child(
            link(s, "sw-favorite", format!("Ajouter « {name} » aux favoris"))
                .on_press(|s: &mut State| s.switcher_add_favorite()),
        );
    }
    actions = actions.child(
        link(s, "sw-overview", "Vue multi-dépôts".to_owned()).on_press_tx(
            |s: &mut State, tx: &mut Tx| {
                s.close_switcher(tx, false);
                s.close_screens();
                s.open_overview();
            },
        ),
    );
    let mut bar = col()
        .key("sw-bar")
        .w_full()
        .px(20.0)
        .py(10.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(input)
        .child(list.h(rows * (ROW_HEIGHT + 2.0)));
    if let Some(notice) = &s.switcher.notice {
        bar = bar.child(
            text(notice.clone())
                .key("sw-notice")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    bar.child(actions)
}
