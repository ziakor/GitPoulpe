use rune::{El, col, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

pub fn init_screen(s: &State) -> El {
    let p = s.palette;
    let mut card = col()
        .gap(10.0)
        .w_full()
        .max_w(520.0)
        .child(
            text("Initialiser un dépôt".to_owned())
                .font(theme::TITLE)
                .color(p.text),
        )
        .child(
            text(s.init.path.display().to_string())
                .key("init-path")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap()
                .selectable(true),
        )
        .child(
            text(
                "Le dossier devient un dépôt Git vide, sans premier commit. Les fichiers qu'il contient restent en place et apparaissent comme non suivis."
                    .to_owned(),
            )
            .font(theme::BODY)
            .color(p.muted)
            .wrap(),
        )
        .child(
            field(
                s,
                "init-branch",
                "Branche initiale",
                &s.init.branch,
                "main",
            )
            .on_edit(|s: &mut State| &mut s.init.branch),
        );
    if let Some(error) = &s.init.error {
        card = card.child(
            text(error.clone())
                .key("init-error")
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        );
    }
    let confirm = if s.init.busy {
        "Initialisation…"
    } else if s.init.error.is_some() {
        "Réessayer"
    } else {
        "Initialiser"
    };
    card = card.child(
        row()
            .gap(8.0)
            .child(
                action(s, "init-confirm", confirm, true).on_press(|s: &mut State| s.confirm_init()),
            )
            .child(
                action(s, "init-cancel", "Annuler", false)
                    .on_press(|s: &mut State| s.cancel_init()),
            ),
    );
    col().w_full().grow(1.0).center().px(16.0).child(card)
}
