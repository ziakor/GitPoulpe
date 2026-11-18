use gitpoulpe_core::{UndoDirection, UndoKind, UndoRecord, UndoState};
use rune::{El, ScrollSpec, col, el, row, text};

use crate::history_state::format_date;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

const ROW_HEIGHT: f32 = 48.0;

fn status(record: &UndoRecord) -> &'static str {
    match (&record.kind, record.state) {
        (UndoKind::Irreversible { .. }, _) => "non réversible",
        (_, UndoState::Done) => "faite",
        (_, UndoState::Undone) => "annulée",
    }
}

fn record_row(s: &State, record: &UndoRecord) -> El {
    let p = s.palette;
    let selected = s.undo_screen.selected == Some(record.id);
    let id = record.id;
    el().key(format!("undo-row{id}"))
        .role("option")
        .label(record.label.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_undo(id))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(record.label.clone())
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(format!(
                        "{} · {} UTC",
                        status(record),
                        format_date(record.at as i64)
                    ))
                    .font(theme::SMALL)
                    .color(if record.reversible() { p.muted } else { p.red })
                    .truncate(),
                ),
        )
}

fn detail(s: &State, records: &[UndoRecord]) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("undo-panel")
        .w(440.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(record) = s
        .undo_screen
        .selected
        .and_then(|id| records.iter().find(|r| r.id == id))
    else {
        return panel.child(
            text("Sélectionnez une opération. Seules celles dont l'inverse est défini peuvent être annulées.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel.child(
        text(record.label.clone())
            .key("undo-selected")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap(),
    );
    if let UndoKind::Irreversible { why } = &record.kind {
        return panel.child(
            text(format!("Cette opération n'est pas réversible ici : {why}."))
                .key("undo-why")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    panel = panel.child(
        text(
            "L'état est relu avant chaque annulation : si un autre outil a changé la branche, la référence ou le nom concernés, rien n'est touché. Vos modifications locales sont conservées quand c'est possible."
                .to_owned(),
        )
        .font(theme::SMALL)
        .color(p.muted)
        .wrap(),
    );
    if let Some(limit) = &record.limit {
        panel = panel.child(
            text(limit.clone())
                .key("undo-limit")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    if let Some(message) = &s.undo_screen.message {
        panel = panel.child(
            text(message.clone())
                .key("undo-message")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    match record.state {
        UndoState::Done => panel.child(
            action(
                s,
                "undo-run",
                if s.undo_screen.busy {
                    "En cours…"
                } else {
                    "Annuler"
                },
                true,
            )
            .on_press(|s: &mut State| s.undo_run(UndoDirection::Undo)),
        ),
        UndoState::Undone => panel.child(
            action(
                s,
                "redo-run",
                if s.undo_screen.busy {
                    "En cours…"
                } else {
                    "Refaire"
                },
                true,
            )
            .on_press(|s: &mut State| s.undo_run(UndoDirection::Redo)),
        ),
    }
}

pub fn undo_screen(s: &State) -> El {
    let p = s.palette;
    let mut records = s.undo_records();
    records.reverse();
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!(
                "Annuler / refaire · {} opération(s)",
                records.len()
            ))
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            action(s, "undo-branches", "Branches", false)
                .on_press(|s: &mut State| s.open_branches()),
        )
        .child(action(s, "undo-close", "Fermer", false).on_press(|s: &mut State| s.close_undo()));
    let list: El = if records.is_empty() {
        col().w_full().grow(1.0).center().child(
            text("Aucune opération enregistrée depuis GitPoulpe pour ce dépôt.".to_owned())
                .key("undo-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        let mut rows = col().key("undo-rows").role("listbox").w_full();
        for record in &records {
            rows = rows.child(record_row(s, record));
        }
        col()
            .key("undo-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(rows)
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(detail(s, &records)),
    )
}
