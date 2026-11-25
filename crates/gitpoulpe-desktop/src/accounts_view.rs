use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{DeviceCode, GithubHost};
use rune::{Color, El, ScrollSpec, col, el, row, text};

use crate::accounts::Status;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, field, quiet, screen_head, toggle};

const COLUMN: f32 = 720.0;

struct Health {
    ink: Color,
    state: String,
    detail: Option<String>,
    advice: Option<String>,
    reconnect: bool,
}

fn on_github_com(connection: &Connection) -> bool {
    connection.provider == Provider::Github && connection.web == GithubHost::dotcom().web
}

fn health(s: &State, connection: &Connection) -> Health {
    let p = s.palette;
    let quiet = |state: &str| Health {
        ink: p.muted,
        state: state.to_owned(),
        detail: None,
        advice: None,
        reconnect: false,
    };
    match s.accounts.statuses.get(&connection.id) {
        None => quiet("Pas encore vérifié"),
        Some(Status::Checking) => quiet("Vérification…"),
        Some(Status::Failed(message)) => Health {
            ink: p.red,
            state: "À reconnecter".to_owned(),
            detail: Some(capitalized(message)),
            advice: None,
            reconnect: true,
        },
        Some(Status::Ok(session)) => {
            let rights = if session.fine_grained {
                "jeton à permissions détaillées".to_owned()
            } else if session.scopes.is_empty() {
                "aucune portée accordée".to_owned()
            } else {
                format!("portées : {}", session.scopes.join(", "))
            };
            let lasting = match (&session.expires_at, session.renews_itself) {
                (_, true) => " · se prolonge automatiquement".to_owned(),
                (Some(date), false) => format!(" · expire le {}", short_date(date)),
                (None, false) => String::new(),
            };
            let expiring = session.expires_at.is_some() && !session.renews_itself;
            let advice = (expiring && on_github_com(connection)).then(|| {
                "Reconnectez-le une fois : il se prolongera ensuite tout seul.".to_owned()
            });
            Health {
                ink: if advice.is_some() { p.lane(1) } else { p.green },
                state: "Connecté".to_owned(),
                detail: Some(format!("{rights}{lasting}")),
                reconnect: advice.is_some(),
                advice,
            }
        }
    }
}

fn short_date(date: &str) -> String {
    match date.strip_suffix(" UTC") {
        Some(stamp) if stamp.len() == 19 => format!("{} UTC", &stamp[..16]),
        _ => date.to_owned(),
    }
}

fn capitalized(message: &str) -> String {
    let mut chars = message.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn dot(color: Color) -> El {
    el().w(8.0).h(8.0).radius(4.0).fill(color)
}

fn badge(s: &State, provider: Provider) -> El {
    let p = s.palette;
    el().px(8.0).py(3.0).radius(3.0).border(1.0, p.edge).child(
        text(provider.caption().to_owned())
            .font(theme::TINY)
            .color(p.text),
    )
}

fn device_steps(s: &State, code: &DeviceCode) -> El {
    let p = s.palette;
    let page = code
        .verification_uri
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let minutes = code.expires_in.div_ceil(60);
    let copied = s.accounts.device_copied;
    let step = |number: &str, body: El| {
        row()
            .w_full()
            .gap(12.0)
            .child(
                el().w(22.0)
                    .h(22.0)
                    .radius(11.0)
                    .center()
                    .border(1.0, p.edge)
                    .child(text(number.to_owned()).font(theme::TINY).color(p.text)),
            )
            .child(col().grow(1.0).min_w(0.0).gap(8.0).child(body))
    };
    col()
        .key("ac-device-box")
        .w_full()
        .gap(12.0)
        .padding(14.0)
        .radius(4.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(step(
            "1",
            col()
                .gap(8.0)
                .child(
                    text("Copiez ce code".to_owned())
                        .font(theme::SMALL)
                        .color(p.muted),
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
                ),
        ))
        .child(step(
            "2",
            col()
                .gap(8.0)
                .child(
                    text(format!(
                        "Collez-le sur {page}, puis autorisez GitPoulpe."
                    ))
                    .key("ac-device-uri")
                    .font(theme::SMALL)
                    .color(p.text)
                    .w_full()
                    .wrap(),
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
                ),
        ))
        .child(
            text(format!(
                "En attente de votre autorisation dans le navigateur… Le code reste valable {minutes} min."
            ))
            .key("ac-device-wait")
            .font(theme::SMALL)
            .color(p.muted)
            .w_full()
            .wrap(),
        )
}

fn account_card(s: &State, index: usize, connection: &Connection) -> El {
    let p = s.palette;
    let health = health(s, connection);
    let (browse_id, check_id, reconnect_id, disconnect_id) = (
        connection.id.clone(),
        connection.id.clone(),
        connection.id.clone(),
        connection.id.clone(),
    );
    let reconnecting = s.accounts.device_for.as_deref() == Some(connection.id.as_str());
    let mut card = col()
        .key(format!("ac-row{index}"))
        .role("group")
        .label(format!(
            "{} {} sur {}",
            connection.provider.caption(),
            connection.login,
            connection.host_caption()
        ))
        .w_full()
        .padding(14.0)
        .gap(10.0)
        .radius(4.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(badge(s, connection.provider))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(connection.login.clone())
                                .font(theme::BODY_STRONG)
                                .color(p.text)
                                .truncate(),
                        )
                        .child(
                            text(connection.host_caption())
                                .font(theme::PATH_SMALL)
                                .color(p.muted)
                                .truncate(),
                        ),
                )
                .child(
                    action(
                        s,
                        &format!("ac-browse{index}"),
                        "Parcourir les dépôts",
                        false,
                    )
                    .on_press(move |s: &mut State| s.open_browser(&browse_id)),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dot(health.ink))
                .child(
                    text(health.state)
                        .key(format!("ac-state{index}"))
                        .font(theme::SMALL)
                        .color(if health.ink == p.red { p.red } else { p.text })
                        .shrink()
                        .wrap(),
                ),
        );
    if let Some(detail) = health.detail {
        card = card.child(
            text(detail)
                .key(format!("ac-detail{index}"))
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .wrap(),
        );
    }
    if let Some(advice) = health.advice {
        card = card.child(
            text(advice)
                .key(format!("ac-advice{index}"))
                .font(theme::SMALL)
                .color(p.text)
                .w_full()
                .wrap(),
        );
    }
    let mut actions = row().gap(6.0).center_y();
    if health.reconnect && !reconnecting {
        actions = actions.child(
            action(s, &format!("ac-reconnect{index}"), "Reconnecter", true)
                .on_press(move |s: &mut State| s.reconnect_account(&reconnect_id)),
        );
    }
    actions = actions
        .child(
            quiet(s, &format!("ac-check{index}"), "Vérifier")
                .on_press(move |s: &mut State| s.check_account(&check_id)),
        )
        .child(
            quiet(s, &format!("ac-disconnect{index}"), "Déconnecter")
                .label(format!("Déconnecter {}", connection.login))
                .on_press(move |s: &mut State| s.disconnect_account(&disconnect_id)),
        );
    card = card.child(actions);
    match &s.accounts.device {
        Some(code) if reconnecting => card.child(device_steps(s, code)),
        _ => card,
    }
}

fn manual_form(s: &State) -> El {
    let p = s.palette;
    let provider = s.accounts.provider;
    let mut form = col()
        .key("ac-manual")
        .w_full()
        .gap(10.0)
        .padding(14.0)
        .radius(4.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    text("Fournisseur".to_owned())
                        .font(theme::SMALL)
                        .color(p.muted)
                        .w(90.0),
                )
                .child(
                    action(s, "ac-prov-github", "GitHub", provider == Provider::Github)
                        .on_press(|s: &mut State| s.set_provider(Provider::Github)),
                )
                .child(
                    action(s, "ac-prov-gitlab", "GitLab", provider == Provider::Gitlab)
                        .on_press(|s: &mut State| s.set_provider(Provider::Gitlab)),
                ),
        )
        .child(
            toggle(
                s,
                "ac-enterprise",
                if provider == Provider::Github {
                    "Serveur GitHub Enterprise"
                } else {
                    "Instance GitLab auto-hébergée"
                },
                s.accounts.enterprise,
            )
            .self_start()
            .px(0.0)
            .on_press(|s: &mut State| {
                let on = !s.accounts.enterprise;
                s.set_enterprise(on)
            }),
        );
    let caption = |words: &str| text(words.to_owned()).font(theme::SMALL).color(p.muted);
    if s.accounts.enterprise {
        form = form.child(caption("Adresse du serveur")).child(
            field(
                s,
                "ac-server",
                "Adresse du serveur",
                &s.accounts.server,
                if provider == Provider::Github {
                    "https://github.exemple.org"
                } else {
                    "https://gitlab.exemple.org"
                },
            )
            .on_edit(|s: &mut State| &mut s.accounts.server),
        );
    }
    form.child(caption("Jeton d'accès personnel")).child(
        field(
            s,
            "ac-token",
            "Jeton d'accès personnel",
            &s.accounts.token,
            "Collez le jeton",
        )
        .secure()
        .on_edit(|s: &mut State| &mut s.accounts.token),
    )
    .child(
        text(if provider == Provider::Github {
            "Jeton classique avec les portées repo et read:org, ou jeton à permissions détaillées. Il ne se prolonge pas tout seul."
        } else {
            "Jeton d'accès personnel GitLab : la portée api permet de commenter et de fusionner, read_api suffit pour lire."
        }
        .to_owned())
        .key("ac-token-hint")
        .font(theme::SMALL)
        .color(p.muted)
        .w_full()
        .wrap(),
    )
    .child(
        row().child(
            action(
                s,
                "ac-connect",
                if s.accounts.busy && s.accounts.device.is_none() {
                    "Connexion…"
                } else {
                    "Connecter"
                },
                true,
            )
            .on_press(|s: &mut State| s.connect_account()),
        ),
    )
}

fn add_section(s: &State) -> El {
    let p = s.palette;
    let mut section = col().w_full().gap(10.0).child(
        text("Ajouter un compte".to_owned())
            .key("ac-connect-title")
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    match &s.accounts.device {
        Some(code) if s.accounts.device_for.is_none() => {
            section = section.child(device_steps(s, code));
        }
        _ => {
            section = section
                .child(
                    row().child(
                        action(
                            s,
                            "ac-device",
                            if s.accounts.busy && s.accounts.device_for.is_none() {
                                "Demande du code…"
                            } else {
                                "Se connecter avec GitHub"
                            },
                            true,
                        )
                        .on_press(|s: &mut State| s.connect_device()),
                    ),
                )
                .child(
                    text("GitHub s'ouvre dans votre navigateur : vous y autorisez GitPoulpe avec un code, sans lui confier de mot de passe. La session se prolonge ensuite toute seule.".to_owned())
                        .key("ac-device-help")
                        .font(theme::SMALL)
                        .color(p.muted)
                        .w_full()
                        .wrap(),
                );
        }
    }
    if let Some(error) = &s.accounts.error {
        section = section.child(
            text(error.clone())
                .key("ac-error")
                .font(theme::SMALL)
                .color(p.red)
                .w_full()
                .wrap(),
        );
    }
    section = section.child(
        quiet(
            s,
            "ac-other",
            if s.accounts.manual {
                "Masquer les autres moyens"
            } else {
                "GitLab, GitHub Enterprise ou jeton d'accès…"
            },
        )
        .self_start()
        .on_press(|s: &mut State| s.toggle_manual()),
    );
    if s.accounts.manual {
        section = section.child(manual_form(s));
    }
    section
}

pub fn accounts_screen(s: &State) -> El {
    if s.accounts.browser.open {
        return crate::repo_browser_view::browser_screen(s);
    }
    let p = s.palette;
    let count = s.accounts.book.connections.len();
    let header = screen_head(
        s,
        "ac-head",
        "Comptes fournisseurs",
        counted(count, "compte connecté", "comptes connectés"),
        vec![action(s, "ac-close", "Fermer", false).on_press(|s: &mut State| s.close_accounts())],
    );
    let mut column = col()
        .w_full()
        .max_w(COLUMN)
        .gap(18.0)
        .child(
            text("GitPoulpe se sert de ces comptes pour les pull requests, les notifications et vos dépôts distants.".to_owned())
                .key("ac-intro")
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .wrap(),
        );
    if let Some(done) = &s.accounts.done {
        column = column.child(
            row()
                .key("ac-ok")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dot(p.green))
                .child(
                    text(done.clone())
                        .font(theme::SMALL)
                        .color(p.text)
                        .shrink()
                        .wrap(),
                ),
        );
    }
    let mut accounts = col().w_full().gap(10.0).child(
        text("Vos comptes".to_owned())
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    if count == 0 {
        accounts = accounts.child(
            text("Aucun compte connecté.".to_owned())
                .key("ac-empty")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    for (index, connection) in s.accounts.book.connections.iter().enumerate() {
        accounts = accounts.child(account_card(s, index, connection));
    }
    column = column
        .child(accounts)
        .child(el().w_full().h(1.0).fill(p.line))
        .child(add_section(s))
        .child(
            text("Les jetons restent dans le trousseau du système, jamais dans les préférences ni dans un dépôt.".to_owned())
                .key("ac-help")
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .wrap(),
        );
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        col()
            .key("ac-panel")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(20.0)
            .py(16.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(column),
    )
}
