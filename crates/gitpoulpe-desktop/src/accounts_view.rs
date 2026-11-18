use std::borrow::Cow;

use gitpoulpe_core::Connection;
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::accounts::Status;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, toggle};

const ROW_HEIGHT: f32 = 52.0;

fn status_line(s: &State, connection: &Connection) -> (String, bool) {
    match s.accounts.statuses.get(&connection.id) {
        None => ("état non vérifié".to_owned(), false),
        Some(Status::Checking) => ("vérification…".to_owned(), false),
        Some(Status::Ok(session)) => (
            if session.fine_grained {
                "connecté · jeton à permissions détaillées".to_owned()
            } else if session.scopes.is_empty() {
                "connecté · aucune portée accordée".to_owned()
            } else {
                format!("connecté · portées : {}", session.scopes.join(", "))
            },
            false,
        ),
        Some(Status::Failed(message)) => (message.clone(), true),
    }
}

fn account_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(connection) = s.accounts.book.connections.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.accounts.selected.as_ref() == Some(&connection.id);
    let (detail, bad) = status_line(s, connection);
    let id = connection.id.clone();
    let mut item = el()
        .key(format!("ac-row{index}"))
        .role("option")
        .label(format!(
            "{} sur {}",
            connection.login,
            connection.host_caption()
        ))
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .on_press(move |s: &mut State| s.select_account(id.clone()));
    if selected {
        item = item.fill(p.panel);
    }
    item.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(
                text(format!(
                    "{} · {}@{}",
                    connection.provider.caption(),
                    connection.login,
                    connection.host_caption()
                ))
                .font(theme::PATH_SMALL)
                .color(p.text)
                .truncate(),
            )
            .child(
                text(detail)
                    .font(theme::SMALL)
                    .color(if bad { p.red } else { p.muted })
                    .truncate(),
            ),
    )
}

fn panel(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("ac-panel")
        .w(460.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if let Some(done) = &s.accounts.done {
        panel = panel.child(
            text(done.clone())
                .key("ac-ok")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &s.accounts.error {
        panel = panel.child(
            text(error.clone())
                .key("ac-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(connection) = s
        .accounts
        .selected
        .as_ref()
        .and_then(|id| s.accounts.book.get(id))
    {
        let id = connection.id.clone();
        panel = panel
            .child(
                text(format!(
                    "{} sur {}",
                    connection.login,
                    connection.host_caption()
                ))
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(
                        action(s, "ac-check", "Vérifier", false)
                            .on_press(move |s: &mut State| s.check_account(&id)),
                    )
                    .child(
                        action(s, "ac-disconnect", "Déconnecter", false)
                            .on_press(|s: &mut State| s.disconnect_account()),
                    ),
            );
    }
    panel = panel
        .child(
            text("Connecter un compte GitHub".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            text("Le jeton d'accès personnel est gardé dans le trousseau du système, jamais dans les préférences ni dans un dépôt. Aucun compte GitPoulpe n'est nécessaire.".to_owned())
                .key("ac-help")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
        .child(
            toggle(
                s,
                "ac-enterprise",
                "GitHub Enterprise Server",
                s.accounts.enterprise,
            )
            .on_press(|s: &mut State| {
                let on = !s.accounts.enterprise;
                s.set_enterprise(on)
            }),
        );
    if s.accounts.enterprise {
        panel = panel.child(
            field(
                s,
                "ac-server",
                "Adresse du serveur",
                &s.accounts.server,
                "https://github.exemple.org",
            )
            .on_edit(|s: &mut State| &mut s.accounts.server),
        );
    }
    panel = panel.child(
        field(
            s,
            "ac-token",
            "Jeton d'accès",
            &s.accounts.token,
            "Collez le jeton",
        )
        .secure()
        .on_edit(|s: &mut State| &mut s.accounts.token),
    );
    panel.child(
        action(
            s,
            "ac-connect",
            if s.accounts.busy {
                "Connexion…"
            } else {
                "Connecter"
            },
            true,
        )
        .on_press(|s: &mut State| s.connect_account()),
    )
}

pub fn accounts_screen(s: &State) -> El {
    let p = s.palette;
    let count = s.accounts.book.connections.len();
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!("Comptes fournisseurs · {count} connexion(s)"))
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(action(s, "ac-close", "Fermer", false).on_press(|s: &mut State| s.close_accounts()));
    let list: El = if count == 0 {
        col().w_full().grow(1.0).center().child(
            text("Aucun compte connecté.".to_owned())
                .key("ac-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("ac-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("ac-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.accounts.version, s.dark))
                    .virtual_rows(
                        count,
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("ack{i}")),
                        account_row,
                    ),
            )
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(panel(s)),
    )
}
