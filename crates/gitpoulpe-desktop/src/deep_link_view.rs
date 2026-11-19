use rune::{El, ScrollSpec, col, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

pub fn deep_link_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.links;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text("Ouvrir un lien".to_owned())
                .key("lk-title")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0),
        )
        .child(
            action(s, "lk-close", "Fermer", false).on_press(|s: &mut State| s.close_deep_link()),
        );
    let mut body = col()
        .key("lk-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .py(8.0)
        .gap(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            field(
                s,
                "lk-input",
                "Lien (gitpoulpe://open/… ou adresse https du dépôt, d'un commit ou d'une demande)",
                &screen.input,
                "https://github.com/propriétaire/dépôt/pull/12",
            )
            .on_edit(|s: &mut State| &mut s.links.input),
        )
        .child(action(s, "lk-analyse", "Analyser", true).on_press(|s: &mut State| s.link_analyse()))
        .child(
            text("Ouvrir un lien ne fait que naviguer : aucune opération Git n'est lancée et aucun compte n'est choisi à votre place.".to_owned())
                .key("lk-hint")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    if let Some(error) = &screen.error {
        body = body.child(
            text(error.clone())
                .key("lk-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if screen.busy {
        body = body.child(
            text("Recherche du dépôt local…".to_owned())
                .key("lk-busy")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(link) = &screen.link {
        body = body.child(
            text(format!("Lien reconnu : {}", link.label()))
                .key("lk-target")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(matches) = &screen.matches {
        if matches.is_empty() {
            body = body.child(
                text("Aucun dépôt local connu (dépôt ouvert, espaces de travail, favoris) ne correspond à ce lien. Clonez-le d'abord, puis rouvrez le lien.".to_owned())
                    .key("lk-none")
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            );
        }
        for (index, path) in matches.iter().enumerate() {
            body = body.child(
                row()
                    .w_full()
                    .center_y()
                    .gap(12.0)
                    .child(
                        text(path.clone())
                            .key(format!("lk-path{index}"))
                            .font(theme::PATH)
                            .color(p.text)
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(
                        action(s, &format!("lk-open{index}"), "Ouvrir ici", true)
                            .on_press(move |s: &mut State| s.link_go(index)),
                    ),
            );
        }
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
