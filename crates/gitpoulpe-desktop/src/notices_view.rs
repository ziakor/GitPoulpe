use gitpoulpe_core::NoticeKind;
use rune::{El, ScrollSpec, col, row, text};

use crate::notices::kind_label;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

pub fn notices_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.notices;
    let prefs = &screen.book.prefs;
    let visible: Vec<_> = screen.book.visible().collect();
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!(
                "Notifications · {} non lue(s){}",
                screen.book.unread(),
                if screen.fetching > 0 {
                    " · lecture…"
                } else {
                    ""
                }
            ))
            .key("nt-title")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            action(s, "nt-refresh", "Actualiser", true)
                .on_press(|s: &mut State| s.notices_refresh()),
        )
        .child(
            action(s, "nt-read-all", "Tout marquer lu", false)
                .on_press(|s: &mut State| s.notices_mark_all_read()),
        )
        .child(action(s, "nt-close", "Fermer", false).on_press(|s: &mut State| s.close_notices()));
    let mut prefs_row = row().w_full().gap(6.0).center_y().child(
        action(
            s,
            "nt-enabled",
            if prefs.enabled {
                "Notifications activées"
            } else {
                "Notifications désactivées"
            },
            prefs.enabled,
        )
        .on_press(|s: &mut State| s.notices_toggle_enabled()),
    );
    for kind in NoticeKind::ALL {
        let muted = prefs.muted_kinds.contains(&kind);
        prefs_row = prefs_row.child(
            action(
                s,
                &format!("nt-kind-{kind:?}").to_lowercase(),
                &format!("{} {}", if muted { "[ ]" } else { "[x]" }, kind.caption()),
                false,
            )
            .on_press(move |s: &mut State| s.notices_toggle_kind(kind)),
        );
    }
    let mut accounts_row = row().w_full().gap(6.0).center_y();
    for (index, connection) in s.accounts.book.connections.iter().enumerate() {
        let id = connection.id.clone();
        let muted = prefs.muted_connections.contains(&id);
        accounts_row = accounts_row.child(
            action(
                s,
                &format!("nt-account{index}"),
                &format!(
                    "{} {}@{}",
                    if muted { "[ ]" } else { "[x]" },
                    connection.login,
                    connection.host_caption()
                ),
                false,
            )
            .on_press(move |s: &mut State| s.notices_toggle_connection(&id)),
        );
    }
    let mut notes = col().w_full().gap(4.0);
    if let Some(message) = &screen.message {
        notes = notes.child(
            text(message.clone())
                .key("nt-message")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    for (connection, why) in &screen.errors {
        let who = s
            .accounts
            .book
            .get(connection)
            .map(|c| format!("{}@{}", c.login, c.host_caption()))
            .unwrap_or_else(|| connection.clone());
        notes = notes.child(
            text(format!("Lecture impossible pour {who} : {why}"))
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    let mut list = col().key("nt-list").w_full().gap(6.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    if visible.is_empty() {
        list = list.child(
            text("Aucune notification.".to_owned())
                .key("nt-empty")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    for (index, notice) in visible.iter().enumerate() {
        let (open_id, dismiss_id) = (notice.id.clone(), notice.id.clone());
        let origin = match &notice.connection {
            Some(id) => s
                .accounts
                .book
                .get(id)
                .map(|c| format!("{}@{}", c.login, c.host_caption()))
                .unwrap_or_else(|| "compte déconnecté".to_owned()),
            None => "local".to_owned(),
        };
        let place = match (&notice.repo, notice.number) {
            (Some(repo), Some(n)) => format!("{repo}#{n}"),
            (Some(repo), None) => repo.clone(),
            _ => notice.repo_path.clone().unwrap_or_default(),
        };
        list = list.child(
            row()
                .key(format!("nt-row{index}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(format!(
                                "{}{}{}",
                                if notice.read { "" } else { "● " },
                                notice.title,
                                if notice.count > 1 {
                                    format!(" (×{})", notice.count)
                                } else {
                                    String::new()
                                }
                            ))
                            .font(if notice.read {
                                theme::BODY
                            } else {
                                theme::BODY_STRONG
                            })
                            .color(p.text)
                            .truncate(),
                        )
                        .child(
                            text(format!(
                                "{} · {place} · {origin} · {}",
                                kind_label(notice.kind),
                                notice.at
                            ))
                            .font(theme::SMALL)
                            .color(p.muted)
                            .truncate(),
                        ),
                )
                .child(
                    action(s, &format!("nt-open{index}"), "Ouvrir", false)
                        .on_press(move |s: &mut State| s.notices_open(&open_id)),
                )
                .child(
                    action(s, &format!("nt-dismiss{index}"), "Écarter", false)
                        .on_press(move |s: &mut State| s.notices_dismiss(&dismiss_id)),
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
            .child(prefs_row)
            .child(accounts_row)
            .child(notes)
            .child(list),
    )
}
