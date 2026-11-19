use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

const ROW_HEIGHT: f32 = 56.0;

fn badges(r: &gitpoulpe_hosting::RepoSummary) -> String {
    let mut parts = Vec::new();
    if r.private {
        parts.push("privé");
    }
    if r.fork {
        parts.push("fork");
    }
    if r.archived {
        parts.push("archivé");
    }
    if !r.can_push {
        parts.push("lecture seule");
    }
    parts.join(" · ")
}

fn scope_item(s: &State, key: String, caption: String, active: bool, scope: Option<String>) -> El {
    let p = s.palette;
    let mut item = el()
        .key(key)
        .role("option")
        .label(caption.clone())
        .w_full()
        .px(12.0)
        .py(7.0)
        .on_press(move |s: &mut State| s.browse_scope(scope.clone()));
    if active {
        item = item.fill(p.panel);
    }
    item.child(text(caption).font(theme::BODY).color(p.text).truncate())
}

fn scopes(s: &State) -> El {
    let p = s.palette;
    let browser = &s.accounts.browser;
    let mut list = col()
        .key("br-scopes")
        .w(230.0)
        .h_full()
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(scope_item(
            s,
            "br-scope-me".to_owned(),
            "Mes dépôts".to_owned(),
            browser.scope.is_none(),
            None,
        ));
    for (i, org) in browser.orgs.iter().enumerate() {
        list = list.child(scope_item(
            s,
            format!("br-scope{i}"),
            org.login.clone(),
            browser.scope.as_deref() == Some(org.login.as_str()),
            Some(org.login.clone()),
        ));
    }
    if let Some(error) = &browser.orgs_error {
        list = list.child(
            text(format!("Organisations indisponibles : {error}"))
                .key("br-orgs-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap()
                .px(12.0)
                .py(8.0),
        );
    }
    list
}

fn repo_row(s: &State, visible: &[usize], index: usize) -> El {
    let p = s.palette;
    let browser = &s.accounts.browser;
    let Some(r) = visible.get(index).and_then(|&i| browser.repos.get(i)) else {
        return el().h(ROW_HEIGHT);
    };
    let name = r.full_name();
    let selected = browser.selected.as_deref() == Some(name.as_str());
    let pressed = name.clone();
    let mut item = el()
        .key(format!("br-row{index}"))
        .role("option")
        .label(name.clone())
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .on_press(move |s: &mut State| s.select_browser_repo(pressed.clone()));
    if selected {
        item = item.fill(p.panel);
    }
    let tags = badges(r);
    let detail = match (&r.description, tags.is_empty()) {
        (Some(d), false) => format!("{tags} · {d}"),
        (Some(d), true) => d.clone(),
        (None, _) => tags,
    };
    item.child(
        col()
            .w_full()
            .h(ROW_HEIGHT)
            .center_y()
            .gap(2.0)
            .child(text(name).font(theme::PATH_SMALL).color(p.text).truncate())
            .child(text(detail).font(theme::SMALL).color(p.muted).truncate()),
    )
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("br-panel")
        .w(320.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(r) = s.accounts.browser.selected_repo() else {
        return panel.child(
            text("Sélectionnez un dépôt pour le cloner.".to_owned())
                .key("br-hint")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel
        .child(
            text(r.full_name())
                .key("br-name")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(
                r.description
                    .clone()
                    .unwrap_or_else(|| "Aucune description.".to_owned()),
            )
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(
            text(format!(
                "Branche par défaut : {}",
                r.default_branch.as_deref().unwrap_or("inconnue")
            ))
            .font(theme::SMALL)
            .color(p.muted),
        )
        .child(
            text(r.clone_url.clone())
                .key("br-url")
                .font(theme::PATH_SMALL)
                .color(p.text)
                .wrap(),
        );
    let tags = badges(r);
    if !tags.is_empty() {
        panel = panel.child(text(tags).key("br-tags").font(theme::SMALL).color(p.muted));
    }
    panel = panel.child(
        action(s, "br-clone", "Cloner ce dépôt", true)
            .on_press(|s: &mut State| s.clone_selected_repo()),
    );
    let browser = &s.accounts.browser;
    if let Some(done) = &browser.fork_done {
        panel = panel.child(
            text(done.clone())
                .key("br-fork-ok")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &browser.fork_error {
        panel = panel.child(
            text(error.clone())
                .key("br-fork-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if browser.forked.is_some() {
        panel = panel.child(
            action(s, "br-clone-fork", "Cloner le fork", false)
                .on_press(|s: &mut State| s.clone_forked_repo()),
        );
    }
    panel = panel.child(
        text(
            if browser.fork_busy {
                "Création du fork en cours…"
            } else {
                "Créer un fork"
            }
            .to_owned(),
        )
        .font(theme::BODY_STRONG)
        .color(p.text),
    );
    if !browser.fork_busy {
        panel = panel.child(
            action(s, "br-fork-me", "Forker vers mon compte", false)
                .on_press(|s: &mut State| s.fork_selected_repo(None)),
        );
        for (i, org) in browser.orgs.iter().enumerate() {
            let login = org.login.clone();
            panel = panel.child(
                action(
                    s,
                    &format!("br-fork{i}"),
                    &format!("Forker vers {}", org.login),
                    false,
                )
                .on_press(move |s: &mut State| s.fork_selected_repo(Some(login.clone()))),
            );
        }
    }
    panel
}

pub fn browser_screen(s: &State) -> El {
    let p = s.palette;
    let browser = &s.accounts.browser;
    let who = browser
        .connection
        .as_ref()
        .and_then(|id| s.accounts.book.get(id))
        .map(|c| format!("{}@{}", c.login, c.host_caption()))
        .unwrap_or_default();
    let visible = browser.visible();
    let count = visible.len();
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!("Dépôts · {who}"))
                .key("br-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            action(s, "br-refresh", "Actualiser", false)
                .on_press(|s: &mut State| s.refresh_browser()),
        )
        .child(action(s, "br-back", "Comptes", false).on_press(|s: &mut State| s.close_browser()));
    let status: Option<(String, bool)> = if browser.loading {
        Some(("Chargement des dépôts…".to_owned(), false))
    } else if let Some(error) = &browser.error {
        Some((error.clone(), true))
    } else if browser.repos.is_empty() {
        Some((
            "Aucun dépôt accessible dans cette portée.".to_owned(),
            false,
        ))
    } else if count == 0 {
        Some(("Aucun dépôt ne correspond au filtre.".to_owned(), false))
    } else if browser.truncated {
        Some((
            "Liste tronquée : trop de dépôts pour tout afficher, affinez par organisation."
                .to_owned(),
            false,
        ))
    } else {
        None
    };
    let mut middle = col().grow(1.0).min_w(0.0).h_full().child(
        el().w_full().px(16.0).py(10.0).child(
            field(
                s,
                "br-filter",
                "Filtrer",
                &browser.filter,
                "Filtrer par nom ou description",
            )
            .on_edit(|s: &mut State| &mut s.accounts.browser.filter),
        ),
    );
    if let Some((message, bad)) = status {
        middle = middle.child(
            text(message)
                .key("br-status")
                .font(theme::SMALL)
                .color(if bad { p.red } else { p.muted })
                .wrap()
                .px(16.0)
                .py(6.0),
        );
    }
    let rows = visible.clone();
    middle = middle.child(
        col()
            .key("br-viewport")
            .grow(1.0)
            .min_w(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("br-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((browser.version, s.dark, browser.filter.text.clone()))
                    .virtual_rows(
                        count,
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("brk{i}")),
                        move |s: &State, i| repo_row(s, &rows, i),
                    ),
            ),
    );
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(scopes(s))
            .child(middle)
            .child(detail(s)),
    )
}
