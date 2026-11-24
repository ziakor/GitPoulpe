use std::borrow::Cow;

use gitpoulpe_core::{Connection, Provider};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::accounts::Status;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, field, quiet, toggle};

const ROW_HEIGHT: f32 = 52.0;

fn status_line(s: &State, connection: &Connection) -> (String, bool) {
    match s.accounts.statuses.get(&connection.id) {
        None => ("état non vérifié".to_owned(), false),
        Some(Status::Checking) => ("vérification…".to_owned(), false),
        Some(Status::Ok(session)) => (
            {
                let base = if session.fine_grained {
                    "connecté · jeton à permissions détaillées".to_owned()
                } else if session.scopes.is_empty() {
                    "connecté · aucune portée accordée".to_owned()
                } else {
                    format!("connecté · portées : {}", session.scopes.join(", "))
                };
                match &session.expires_at {
                    Some(date) => format!("{base} · expire le {date}"),
                    None => base,
                }
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
                        action(s, "ac-browse", "Parcourir les dépôts", true).on_press({
                            let id = connection.id.clone();
                            move |s: &mut State| s.open_browser(&id)
                        }),
                    )
                    .child(
                        action(s, "ac-disconnect", "Déconnecter", false)
                            .on_press(|s: &mut State| s.disconnect_account()),
                    ),
            );
    }
    let provider = s.accounts.provider;
    panel = panel
        .child(
            text(format!("Connecter un compte {}", provider.caption()))
                .key("ac-connect-title")
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    action(s, "ac-prov-github", "GitHub", provider == Provider::Github)
                        .on_press(|s: &mut State| s.set_provider(Provider::Github)),
                )
                .child(
                    action(s, "ac-prov-gitlab", "GitLab", provider == Provider::Gitlab)
                        .on_press(|s: &mut State| s.set_provider(Provider::Gitlab)),
                ),
        );
    if let Some(code) = &s.accounts.device {
        let page = code
            .verification_uri
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let minutes = code.expires_in.div_ceil(60);
        let copied = s.accounts.device_copied;
        panel = panel.child(
            col()
                .key("ac-device-box")
                .w_full()
                .gap(10.0)
                .padding(14.0)
                .radius(3.0)
                .fill(p.panel)
                .border(1.0, p.line)
                .child(
                    text(format!("Saisissez ce code sur {page} :"))
                        .key("ac-device-uri")
                        .font(theme::SMALL)
                        .color(p.text)
                        .w_full()
                        .wrap(),
                )
                .child(
                    row()
                        .gap(14.0)
                        .center_y()
                        .child(
                            text(code.user_code.clone())
                                .key("ac-device-code")
                                .font(theme::CODE_LARGE)
                                .color(p.text)
                                .selectable(true),
                        )
                        .child(
                            action(
                                s,
                                "ac-device-copy",
                                if copied { "Code copié" } else { "Copier le code" },
                                false,
                            )
                            .on_press_tx(|s: &mut State, tx| s.copy_device_code(tx, false)),
                        ),
                )
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(
                            action(s, "ac-device-open", "Copier le code et ouvrir GitHub", true)
                                .on_press_tx(|s: &mut State, tx| s.copy_device_code(tx, true)),
                        )
                        .child(
                            quiet(s, "ac-device-cancel", "Annuler")
                                .on_press(|s: &mut State| s.cancel_device()),
                        ),
                )
                .child(
                    text(format!(
                        "En attente de votre autorisation dans le navigateur… Le code reste valable {minutes} min."
                    ))
                    .key("ac-device-wait")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .w_full()
                    .wrap(),
                ),
        );
    } else if !s.accounts.enterprise && provider == Provider::Github {
        panel = panel.child(
            action(
                s,
                "ac-device",
                if s.accounts.busy {
                    "Demande du code…"
                } else {
                    "Se connecter avec GitHub"
                },
                true,
            )
            .on_press(|s: &mut State| s.connect_device()),
        );
    }
    panel = panel
        .child(
            text("Le jeton est gardé dans le trousseau du système, jamais dans les préférences ni dans un dépôt. Sans navigateur, pour une instance privée ou pour GitLab, collez un jeton d'accès personnel.".to_owned())
                .key("ac-help")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
        .child(
            toggle(
                s,
                "ac-enterprise",
                if provider == Provider::Github {
                    "GitHub Enterprise Server"
                } else {
                    "Instance GitLab auto-hébergée"
                },
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
    if s.accounts.browser.open {
        return crate::repo_browser_view::browser_screen(s);
    }
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
