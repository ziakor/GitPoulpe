use std::borrow::Cow;

use gitpoulpe_core::mask_url;
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

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

fn selected_detail(s: &State, mut panel: El) -> El {
    let p = s.palette;
    let Some(remote) = s.remotes.selected_remote() else {
        return panel.child(
            text("Sélectionne un remote pour le modifier.".to_owned())
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
                .key("rm-fetch")
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
                        "Supprimer « {} » retire {} branche(s) de suivi ; {branches}. Les branches locales et les commits restent. Pour le rétablir : {}",
                        remote.name,
                        impact.tracking_refs,
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
                                action(s, "rm-remove-confirm", "Supprimer le remote", true)
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
            action(s, "rm-remove", "Supprimer ce remote", false)
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
            text("Nouveau remote".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            field(
                s,
                "rm-new-name",
                "Nom du remote",
                &s.remotes.new_name,
                "origin",
            )
            .on_edit(|s: &mut State| &mut s.remotes.new_name),
        )
        .child(
            field(
                s,
                "rm-new-url",
                "URL du remote",
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
    let title = if s.remotes.loading {
        "Remotes · lecture…".to_owned()
    } else {
        format!("Remotes · {}", s.remotes.remotes.len())
    };
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(title)
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(s, "remotes-branches", "Branches", false)
                .on_press(|s: &mut State| s.open_branches()),
        )
        .child(
            action(s, "remotes-close", "Fermer", false).on_press(|s: &mut State| s.close_remotes()),
        );
    let list: El = if let Some(error) = &s.remotes.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.remotes.remotes.is_empty() && !s.remotes.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun remote dans ce dépôt.".to_owned())
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
