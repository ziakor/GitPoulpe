use gitpoulpe_core::Favorite;
use rune::{El, ScrollSpec, col, row, text, text_input};

use crate::favorites::RemoteAccess;
use crate::state::State;
use crate::theme;
use crate::widgets::action;
use crate::workspaces::{RepoHealth, health};

pub fn favorites_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.favorites;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!("Favoris · {}", screen.book.items.len()))
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0),
        )
        .child(
            action(s, "fv-add-current", "Ajouter le dépôt courant", false)
                .on_press(|s: &mut State| s.favorites_add_current()),
        )
        .child(
            action(s, "fv-accounts", "Comptes fournisseurs", false)
                .on_press(|s: &mut State| s.open_accounts()),
        )
        .child(
            action(s, "fv-close", "Fermer", false).on_press(|s: &mut State| s.close_favorites()),
        );
    let search = text_input(&screen.query, theme::BODY)
        .key("fv-search")
        .label("Rechercher un favori")
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder("Rechercher un dépôt favori…".to_owned(), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
        .on_edit(|s: &mut State| &mut s.favorites.query);
    let mut notes = col().w_full().gap(4.0);
    if let Some(error) = &screen.error {
        notes = notes.child(
            text(error.clone())
                .key("fv-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &screen.notice {
        notes = notes.child(
            text(notice.clone())
                .key("fv-notice")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    let mut list = col().key("fv-list").w_full().gap(6.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    let visible = s.favorites_visible();
    if visible.is_empty() {
        list = list.child(
            text(if screen.book.items.is_empty() {
                "Aucun favori : ajoutez le dépôt courant ou un dépôt depuis l'explorateur de comptes."
            } else {
                "Aucun favori ne correspond à cette recherche."
            }
            .to_owned())
            .key("fv-empty")
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        );
    }
    for (index, favorite) in visible.iter().enumerate() {
        let key = favorite.key();
        let (kind, detail, broken) = match favorite {
            Favorite::Local { path, .. } => {
                let state = health(path);
                (
                    "référence locale",
                    format!(
                        "{path} · {}",
                        match state {
                            RepoHealth::Ok => "accessible",
                            RepoHealth::Missing => "introuvable",
                            RepoHealth::NotRepository => "n'est plus un dépôt Git",
                        }
                    ),
                    state != RepoHealth::Ok,
                )
            }
            Favorite::Remote {
                connection,
                clone_url,
                ..
            } => {
                let (word, broken) = match s.favorite_remote_access(connection) {
                    RemoteAccess::NoAccount => ("compte déconnecté".to_owned(), true),
                    RemoteAccess::Unchecked(who) => {
                        (format!("accès via {who}, non vérifié"), false)
                    }
                    RemoteAccess::Checking(who) => {
                        (format!("accès via {who}, vérification…"), false)
                    }
                    RemoteAccess::Verified(who) => (format!("accès via {who}, vérifié"), false),
                    RemoteAccess::Revoked(who, _) => (format!("accès de {who} refusé"), true),
                };
                ("accès fournisseur", format!("{clone_url} · {word}"), broken)
            }
        };
        let is_local = matches!(favorite, Favorite::Local { .. });
        let (open_key, remove_key, repair_key, check_key) =
            (key.clone(), key.clone(), key.clone(), key.clone());
        let mut line = row()
            .key(format!("fv-row{index}"))
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(format!("{} · {kind}", favorite.label()))
                            .font(theme::BODY_STRONG)
                            .color(p.text)
                            .truncate(),
                    )
                    .child(
                        text(detail)
                            .font(theme::SMALL)
                            .color(if broken { p.red } else { p.muted })
                            .truncate(),
                    ),
            )
            .child(
                action(
                    s,
                    &format!("fv-open{index}"),
                    if is_local { "Ouvrir" } else { "Cloner" },
                    false,
                )
                .on_press(move |s: &mut State| s.favorite_open(&open_key)),
            );
        if is_local && broken {
            line = line.child(
                action(s, &format!("fv-repair{index}"), "Réparer…", true).on_press_tx(
                    move |s: &mut State, tx| s.favorite_repair_folder(repair_key.clone(), tx),
                ),
            );
        }
        if !is_local {
            line = line.child(
                action(s, &format!("fv-check{index}"), "Vérifier l'accès", false)
                    .on_press(move |s: &mut State| s.favorite_check(&check_key)),
            );
        }
        line = line.child(
            action(s, &format!("fv-remove{index}"), "Retirer", false)
                .on_press(move |s: &mut State| s.favorite_remove(&remove_key)),
        );
        list = list.child(line);
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
