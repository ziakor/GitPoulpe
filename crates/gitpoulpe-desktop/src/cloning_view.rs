use rune::{El, col, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

pub fn clone_screen(s: &State) -> El {
    let p = s.palette;
    let proposed = s.clone.proposed_name();
    let mut card = col()
        .gap(10.0)
        .w_full()
        .max_w(560.0)
        .child(
            text("Cloner un dépôt".to_owned())
                .font(theme::TITLE)
                .color(p.text),
        )
        .child(
            field(
                s,
                "clone-url",
                "URL du dépôt",
                &s.clone.url,
                "https://exemple.org/depot.git ou git@hote:depot.git",
            )
            .on_edit(|s: &mut State| &mut s.clone.url),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    text(s.clone.parent.as_ref().map_or_else(
                        || "Aucun dossier choisi".to_owned(),
                        |d| d.display().to_string(),
                    ))
                    .key("clone-parent-label")
                    .font(theme::PATH_SMALL)
                    .color(p.muted)
                    .grow(1.0)
                    .truncate(),
                )
                .child(
                    action(s, "clone-parent", "Choisir le dossier…", false).on_press_tx(
                        |_: &mut State, tx: &mut rune::Tx| crate::keys::clone_parent_dialog(tx),
                    ),
                ),
        )
        .child(
            field(
                s,
                "clone-name",
                "Nom du dossier",
                &s.clone.name,
                if proposed.is_empty() {
                    "nom du dossier"
                } else {
                    &proposed
                },
            )
            .on_edit(|s: &mut State| &mut s.clone.name),
        );
    if let Some((phase, percent)) = &s.clone.progress {
        card = card.child(
            text(format!("Clonage · {phase} {percent} %"))
                .key("clone-progress")
                .font(theme::BODY)
                .color(p.muted),
        );
    }
    if let Some(error) = &s.clone.error {
        card = card.child(
            text(error.clone())
                .key("clone-error")
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        );
    }
    let buttons = if s.clone.busy {
        row().gap(8.0).child(
            action(s, "clone-stop", "Annuler le clonage", false)
                .on_press(|s: &mut State| s.cancel_clone()),
        )
    } else {
        row()
            .gap(8.0)
            .child(
                action(
                    s,
                    "clone-start",
                    if s.clone.error.is_some() {
                        "Réessayer"
                    } else {
                        "Cloner"
                    },
                    true,
                )
                .on_press(|s: &mut State| s.start_clone()),
            )
            .child(
                action(s, "clone-close", "Fermer", false).on_press(|s: &mut State| s.close_clone()),
            )
    };
    col()
        .w_full()
        .grow(1.0)
        .center()
        .px(16.0)
        .child(card.child(buttons))
}
