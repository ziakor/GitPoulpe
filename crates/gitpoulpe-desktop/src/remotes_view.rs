use std::borrow::Cow;

use gitpoulpe_core::mask_url;
use gitpoulpe_git::NetError;
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::remotes::{CheckState, CheckStatus, FetchStatus, ScanStatus};
use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, field, screen_head};

const ROW_HEIGHT: f32 = 48.0;

fn remote_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(remote) = s.remotes.remotes.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.remotes.selected.as_deref() == Some(remote.name.as_str());
    let name = remote.name.clone();
    let fetch = remote
        .fetch_urls
        .first()
        .map_or_else(|| "aucune URL".to_owned(), |u| mask_url(u));
    let detail = if remote.push_urls.is_empty() {
        fetch
    } else {
        format!("{fetch} · envoi différent")
    };
    el().key(format!("rm-row{index}"))
        .role("option")
        .label(remote.name.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_remote(&name))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(remote.name.clone())
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::PATH_SMALL)
                        .color(p.muted)
                        .truncate(),
                ),
        )
}

fn urls_line(label: &str, urls: &[String]) -> String {
    let shown: Vec<String> = urls.iter().map(|u| mask_url(u)).collect();
    format!("{label} : {}", shown.join(", "))
}

fn check_section(s: &State, mut panel: El, remote_name: &str) -> El {
    let p = s.palette;
    let state: Option<&CheckState> = s.remotes.check.as_ref().filter(|c| c.name == remote_name);
    let running = matches!(state.map(|c| &c.status), Some(CheckStatus::Running));
    panel = panel.child(el().h(1.0).w_full().fill(p.line));
    if let Some(fetch) = s.remotes.fetch.as_ref().filter(|f| f.remote == remote_name) {
        match &fetch.status {
            FetchStatus::Running { phase, percent } => {
                return panel
                    .child(
                        text(format!("Récupération · {phase} {percent} %"))
                            .key("rm-fetch-running")
                            .font(theme::SMALL)
                            .color(p.muted),
                    )
                    .child(
                        action(s, "rm-check-cancel", "Annuler la récupération", false)
                            .on_press(|s: &mut State| s.cancel_remote_check()),
                    );
            }
            FetchStatus::Done(outcome) => {
                let news = outcome.updates.iter().filter(|u| u.is_new()).count();
                panel = panel.child(
                    text(if outcome.updates.is_empty() {
                        "Récupération terminée : rien de nouveau. Votre travail local n'a pas été touché."
                            .to_owned()
                    } else {
                        format!(
                            "Récupération terminée : {} à jour dont {}. Votre travail local n'a pas été touché.",
                            counted(outcome.updates.len(), "référence mise", "références mises"),
                            counted(news, "nouvelle", "nouvelles")
                        )
                    })
                    .key("rm-fetch-ok")
                    .font(theme::SMALL)
                    .color(p.text)
                    .wrap(),
                );
            }
            FetchStatus::Failed(error) => {
                panel = panel
                    .child(
                        text(format!("Récupération impossible : {error}"))
                            .key("rm-fetch-error")
                            .font(theme::SMALL)
                            .color(p.red)
                            .wrap(),
                    )
                    .child(
                        text(
                            "Les références déjà connues sont intactes ; vous pouvez relancer."
                                .to_owned(),
                        )
                        .font(theme::SMALL)
                        .color(p.muted)
                        .wrap(),
                    );
            }
        }
    }
    panel = panel.child(
        action(s, "rm-fetch", "Récupérer (fetch)", true).on_press(|s: &mut State| s.remote_fetch()),
    );
    if running {
        return panel
            .child(
                text("Test de l'accès en cours…".to_owned())
                    .key("rm-check-running")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                action(s, "rm-check-cancel", "Annuler le test", false)
                    .on_press(|s: &mut State| s.cancel_remote_check()),
            );
    }
    panel = panel.child(
        action(s, "rm-check", "Tester l'accès", false).on_press(|s: &mut State| s.remote_check()),
    );
    let Some(state) = state else {
        return panel;
    };
    match &state.status {
        CheckStatus::Running => panel,
        CheckStatus::Reachable(check) => panel.child(
            text(format!(
                "Accès réussi : {}, {}{}.",
                counted(check.branches, "branche", "branches"),
                counted(check.tags, "tag", "tags"),
                check
                    .head
                    .as_deref()
                    .map_or_else(String::new, |h| format!(", HEAD sur {h}"))
            ))
            .key("rm-check-ok")
            .font(theme::SMALL)
            .color(p.text)
            .wrap(),
        ),
        CheckStatus::Failed(error) => {
            panel = panel.child(
                text(format!("Accès impossible : {error}"))
                    .key("rm-check-error")
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
            match error {
                NetError::SshHostUnknown { .. } => host_decision(s, panel, state),
                NetError::SshHostChanged { host } if !host.is_empty() => panel.child(
                    text(format!(
                        "Si ce changement est légitime, retirez l'ancienne entrée vous-même puis réessayez : ssh-keygen -R {}",
                        crate::shell::quote(host)
                    ))
                    .key("rm-host-changed")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap()
                    .selectable(true),
                ),
                _ => panel,
            }
        }
    }
}

fn host_decision(s: &State, mut panel: El, state: &CheckState) -> El {
    let p = s.palette;
    match &state.scan {
        None => panel.child(
            action(s, "rm-scan", "Voir l'empreinte de l'hôte", false)
                .on_press(|s: &mut State| s.remote_scan_host()),
        ),
        Some(ScanStatus::Scanning) => panel.child(
            text("Lecture de l'empreinte…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        ),
        Some(ScanStatus::Failed(error)) => panel.child(
            text(format!("Empreinte illisible : {error}"))
                .key("rm-scan-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        ),
        Some(ScanStatus::Keys(keys)) => {
            panel = panel.child(
                text(
                    "Comparez ces empreintes avec celles publiées par le fournisseur avant d'accepter."
                        .to_owned(),
                )
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
            );
            for (index, key) in keys.iter().enumerate() {
                panel = panel
                    .child(
                        text(format!("{} {}", key.key_type, key.fingerprint))
                            .key(format!("rm-fp{index}"))
                            .font(theme::PATH_SMALL)
                            .color(p.text)
                            .wrap()
                            .selectable(true),
                    )
                    .child(
                        action(
                            s,
                            &format!("rm-trust{index}"),
                            "Faire confiance à cette empreinte",
                            false,
                        )
                        .on_press(move |s: &mut State| s.remote_trust_host(index)),
                    );
            }
            panel
        }
    }
}

fn selected_detail(s: &State, mut panel: El) -> El {
    let p = s.palette;
    let Some(remote) = s.remotes.selected_remote() else {
        return panel.child(
            text("Sélectionnez un dépôt distant pour le modifier.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel
        .child(el().h(1.0).w_full().fill(p.line))
        .child(
            text(remote.name.clone())
                .key("rm-selected")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap()
                .selectable(true),
        )
        .child(
            text(urls_line("Lecture", &remote.fetch_urls))
                .key("rm-urls-read")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap()
                .selectable(true),
        )
        .child(
            text(urls_line("Envoi", remote.push_targets()))
                .key("rm-push")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap()
                .selectable(true),
        );
    if remote.has_credentials() {
        panel = panel.child(
            text(
                "Cette URL contient des identifiants : ils ne sont jamais affichés ni préremplis."
                    .to_owned(),
            )
            .key("rm-secret")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    panel = panel
        .child(
            field(
                s,
                "rm-url",
                "Nouvelle URL de lecture",
                &s.remotes.url,
                "Nouvelle URL de lecture",
            )
            .on_edit(|s: &mut State| &mut s.remotes.url),
        )
        .child(
            action(s, "rm-url-save", "Changer l'URL de lecture", false)
                .on_press(|s: &mut State| s.remote_set_url()),
        )
        .child(
            field(
                s,
                "rm-push-url",
                "Nouvelle URL d'envoi",
                &s.remotes.push_url,
                "Nouvelle URL d'envoi",
            )
            .on_edit(|s: &mut State| &mut s.remotes.push_url),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    action(s, "rm-push-save", "Changer l'URL d'envoi", false)
                        .on_press(|s: &mut State| s.remote_set_push_url(false)),
                )
                .child(
                    action(s, "rm-push-clear", "Envoyer vers l'URL de lecture", false)
                        .on_press(|s: &mut State| s.remote_set_push_url(true)),
                ),
        )
        .child(
            field(
                s,
                "rm-rename",
                "Nouveau nom",
                &s.remotes.rename,
                "Nouveau nom",
            )
            .on_edit(|s: &mut State| &mut s.remotes.rename),
        )
        .child(
            action(s, "rm-rename-save", "Renommer", false)
                .on_press(|s: &mut State| s.remote_rename()),
        );
    panel = check_section(s, panel, &remote.name);
    match &s.remotes.confirm_remove {
        Some((name, impact)) if *name == remote.name => {
            let consequence = match impact {
                None => "Calcul de l'impact…".to_owned(),
                Some(impact) => {
                    let branches = if impact.upstream_of.is_empty() {
                        "aucune branche locale ne le suit".to_owned()
                    } else {
                        format!(
                            "{} perdra son suivi : {}",
                            if impact.upstream_of.len() == 1 {
                                "la branche locale"
                            } else {
                                "les branches locales"
                            },
                            impact.upstream_of.join(", ")
                        )
                    };
                    let url = remote.fetch_urls.first().cloned().unwrap_or_default();
                    format!(
                        "Supprimer « {} » retire {} ; {branches}. Les branches locales et les commits restent. Pour le rétablir : {}",
                        remote.name,
                        counted(
                            impact.tracking_refs,
                            "branche de suivi",
                            "branches de suivi"
                        ),
                        crate::shell::restore_remote(&remote.name, &url)
                    )
                }
            };
            panel.child(
                col()
                    .key("rm-confirm")
                    .w_full()
                    .gap(6.0)
                    .px(12.0)
                    .py(10.0)
                    .radius(3.0)
                    .border(1.0, p.red)
                    .child(text(consequence).font(theme::SMALL).color(p.red).wrap())
                    .child(
                        row()
                            .gap(8.0)
                            .child(
                                action(s, "rm-remove-confirm", "Supprimer le dépôt distant", true)
                                    .on_press(|s: &mut State| s.remote_remove()),
                            )
                            .child(
                                action(s, "rm-remove-cancel", "Annuler", false)
                                    .on_press(|s: &mut State| s.remote_cancel_remove()),
                            ),
                    ),
            )
        }
        _ => panel.child(
            action(s, "rm-remove", "Supprimer ce dépôt distant", false)
                .on_press(|s: &mut State| s.remote_ask_remove()),
        ),
    }
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let panel = col()
        .key("rm-panel")
        .w(400.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text("Nouveau dépôt distant".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "rm-new-name",
                "Nom du dépôt distant",
                &s.remotes.new_name,
                "origin",
            )
            .on_edit(|s: &mut State| &mut s.remotes.new_name),
        )
        .child(
            field(
                s,
                "rm-new-url",
                "URL du dépôt distant",
                &s.remotes.new_url,
                "https://exemple.org/depot.git",
            )
            .on_edit(|s: &mut State| &mut s.remotes.new_url),
        )
        .child(action(s, "rm-add", "Ajouter", true).on_press(|s: &mut State| s.remote_add()));
    selected_detail(s, panel)
}

pub fn remotes_screen(s: &State) -> El {
    let p = s.palette;
    let context = if s.remotes.loading {
        "Lecture…".to_owned()
    } else {
        counted(s.remotes.remotes.len(), "dépôt distant", "dépôts distants")
    };
    let header = screen_head(s, "remotes-head", "Dépôts distants", context, Vec::new());
    let list: El = if let Some(error) = &s.remotes.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.remotes.remotes.is_empty() && !s.remotes.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun dépôt distant dans ce dépôt.".to_owned())
                .key("remotes-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("remotes-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("remotes-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.remotes.version, s.dark))
                    .virtual_rows(
                        s.remotes.remotes.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("rmk{i}")),
                        remote_row,
                    ),
            )
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(detail(s)),
    )
}
