use rune::{El, col, row, text, text_input};

use crate::state::State;
use crate::theme;
use crate::widgets::action;

fn notice(s: &State, key: &'static str, message: String, error: bool) -> El {
    let p = s.palette;
    text(message)
        .key(key)
        .font(theme::SMALL)
        .color(if error { p.red } else { p.muted })
        .wrap()
}

pub fn file_edit_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.file_edit;
    let dirty = screen.dirty();
    let title = format!(
        "Édition de {}{}",
        screen.path,
        if dirty {
            " · modifications non enregistrées"
        } else {
            ""
        }
    );
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(title)
                .key("fe-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(
                s,
                "fe-save",
                if screen.saving {
                    "Enregistrement…"
                } else {
                    "Enregistrer"
                },
                dirty,
            )
            .on_press(|s: &mut State| s.file_edit_save(false)),
        )
        .child(
            action(s, "fe-close", "Fermer", false)
                .on_press(|s: &mut State| s.file_edit_close_request()),
        );
    let mut notes = col().w_full().px(20.0).gap(6.0);
    if let Some(message) = &screen.notice {
        notes = notes.child(notice(s, "fe-notice", message.clone(), false));
    }
    if screen.loading {
        notes = notes.child(notice(
            s,
            "fe-loading",
            "Lecture du fichier…".to_owned(),
            false,
        ));
    }
    if let Some(message) = &screen.save_error {
        notes = notes.child(notice(s, "fe-save-error", message.clone(), true));
    }
    if screen.external {
        notes = notes.child(
            row()
                .gap(8.0)
                .child(
                    action(s, "fe-overwrite", "Écraser avec mon texte", false)
                        .on_press(|s: &mut State| s.file_edit_save(true)),
                )
                .child(
                    action(
                        s,
                        "fe-reload",
                        "Recharger depuis le disque (abandonner mon texte)",
                        false,
                    )
                    .on_press(|s: &mut State| s.file_edit_reload_from_disk()),
                )
                .child(
                    action(s, "fe-keep", "Garder mon texte", false)
                        .on_press(|s: &mut State| s.file_edit_keep_mine()),
                ),
        );
    }
    if screen.confirm_discard {
        notes = notes
            .child(notice(
                s,
                "fe-discard-warning",
                "Des modifications ne sont pas enregistrées : les abandonner ?".to_owned(),
                true,
            ))
            .child(
                row()
                    .gap(8.0)
                    .child(
                        action(s, "fe-discard", "Abandonner les modifications", false)
                            .on_press(|s: &mut State| s.close_file_edit()),
                    )
                    .child(
                        action(s, "fe-continue", "Continuer l'édition", true)
                            .on_press(|s: &mut State| s.file_edit_cancel_close()),
                    ),
            );
    }
    let body: El = if let Some(error) = &screen.load_error {
        col().w_full().grow(1.0).center().child(
            text(format!("Édition impossible : {error}"))
                .key("fe-load-error")
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if screen.original.is_none() {
        col().w_full().grow(1.0)
    } else {
        let note = screen.original.as_ref().map(|o| {
            format!(
                "{}{}",
                match o.line_ending {
                    gitpoulpe_git::LineEnding::Lf => "Fins de ligne : LF",
                    gitpoulpe_git::LineEnding::Crlf =>
                        "Fins de ligne : CRLF (conservées à l'enregistrement)",
                },
                if o.bom { " · BOM UTF-8 conservé" } else { "" }
            )
        });
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(20.0)
            .py(8.0)
            .gap(6.0)
            .child(
                text(note.unwrap_or_default())
                    .key("fe-format")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                text_input(&screen.text, theme::CODE)
                    .key("fe-editor")
                    .label("Contenu du fichier")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .px(10.0)
                    .py(8.0)
                    .radius(3.0)
                    .fill(p.raised)
                    .border(1.0, p.line)
                    .color(p.text)
                    .focus_ring(2.0, 2.0, p.accent)
                    .on_edit(|s: &mut State| &mut s.file_edit.text)
                    .on_input_key(|s: &mut State, event, _| {
                        if event.mods.cmd && event.is_command('s') {
                            s.file_edit_save(false);
                            return true;
                        }
                        if event.key == rune::Key::Escape {
                            s.file_edit_close_request();
                            return true;
                        }
                        false
                    }),
            )
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(notes)
        .child(body)
}
