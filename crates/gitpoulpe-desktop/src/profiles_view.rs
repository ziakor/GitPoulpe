use gitpoulpe_core::display_label;
use gitpoulpe_git::SigningCheck;
use rune::{El, col, el, row, text, text_input};

use crate::profiles::SigningKind;
use crate::state::State;
use crate::theme;

fn button(s: &State, key: String, label: String, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key)
        .role("button")
        .label(label.clone())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .child(text(label).font(theme::BODY).color(ink))
}

fn labelled(s: &State, caption: &str, input: El) -> El {
    col()
        .w_full()
        .gap(4.0)
        .child(
            text(caption.to_owned())
                .font(theme::SMALL)
                .color(s.palette.muted),
        )
        .child(input)
}

fn field(s: &State, key: &str, label: &str, edit: &rune::TextEdit, placeholder: &str) -> El {
    let p = s.palette;
    text_input(edit, theme::BODY)
        .key(key.to_owned())
        .label(label.to_owned())
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(placeholder.to_owned(), p.muted)
        .focus_ring(2.0, 2.0, p.accent)
}

fn choice(s: &State, key: &str, caption: &str, kind: SigningKind) -> El {
    let selected = s.form.signing == kind;
    button(s, key.to_owned(), caption.to_owned(), selected)
        .on_press(move |s: &mut State| s.set_form_signing(kind))
}

fn signing_status(s: &State) -> Option<El> {
    let p = s.palette;
    let check = s.form.signing_check.as_ref()?;
    let (message, ink) = match check {
        SigningCheck::NotRequested => return None,
        SigningCheck::Ready => ("Référence de clé valide.".to_owned(), p.green),
        SigningCheck::Problem(message) => (message.clone(), p.red),
    };
    Some(text(message).font(theme::SMALL).color(ink).wrap())
}

fn identity_box(s: &State) -> El {
    let p = s.palette;
    let mut card = col()
        .w_full()
        .gap(4.0)
        .px(14.0)
        .py(10.0)
        .fill(p.panel)
        .border(1.0, p.line);
    card = card.child(
        text("Identité effective pour ce dépôt".to_owned())
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    let Some(shown) = &s.identity_shown else {
        return card.child(
            text("Calcul en cours…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    let origin = shown
        .profile
        .as_ref()
        .map_or("configuration Git".to_owned(), |label| {
            format!("profil « {label} »")
        });
    match &shown.result {
        Ok(identity) => {
            card = card.child(
                text(format!("{} <{}>", identity.name, identity.email))
                    .font(theme::BODY)
                    .color(p.text),
            );
        }
        Err(message) => {
            card = card.child(
                text(format!(
                    "Aucune identité : {}",
                    message.lines().next().unwrap_or("")
                ))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
            );
        }
    }
    card = card.child(
        text(format!("Source : {origin}"))
            .font(theme::SMALL)
            .color(p.muted),
    );
    match &shown.signing {
        SigningCheck::NotRequested => card.child(
            text("Signature : selon la configuration Git.".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        ),
        SigningCheck::Ready => card.child(
            text("Signature : la clé du profil est utilisable.".to_owned())
                .font(theme::SMALL)
                .color(p.green),
        ),
        SigningCheck::Problem(message) => card.child(
            text(format!("Signature impossible : {message}"))
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        ),
    }
}

pub fn profiles_screen(s: &State) -> El {
    let p = s.palette;
    let mut list = col().w(240.0).gap(6.0);
    for profile in &s.book.profiles {
        let id = profile.id.clone();
        let selected = s.form.editing.as_deref() == Some(profile.id.as_str());
        let used = s.active_profile().is_some_and(|a| a.id == profile.id);
        let caption = if used {
            format!("{} · ce dépôt", profile.label)
        } else {
            profile.label.clone()
        };
        list = list.child(
            button(s, format!("pf-{}", profile.id), caption, selected)
                .w_full()
                .on_press(move |s: &mut State| s.edit_profile(&id)),
        );
    }
    list = list.child(
        button(s, "pf-new".to_owned(), "Nouveau profil".to_owned(), false)
            .w_full()
            .on_press(|s: &mut State| s.new_profile()),
    );

    let mut form = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .max_w(560.0)
        .child(labelled(
            s,
            "Nom du profil",
            field(
                s,
                "pf-label",
                "Nom du profil",
                &s.form.label,
                "Perso, Travail…",
            )
            .on_edit(|s: &mut State| &mut s.form.label),
        ))
        .child(labelled(
            s,
            "Nom Git (user.name)",
            field(s, "pf-name", "Nom Git", &s.form.name, "Ada Lovelace")
                .on_edit(|s: &mut State| &mut s.form.name),
        ))
        .child(labelled(
            s,
            "E-mail (user.email)",
            field(s, "pf-email", "E-mail", &s.form.email, "ada@example.org")
                .on_edit(|s: &mut State| &mut s.form.email),
        ))
        .child(labelled(
            s,
            "Signature des commits",
            row()
                .gap(8.0)
                .child(choice(
                    s,
                    "pf-sign-none",
                    "Selon la configuration Git",
                    SigningKind::None,
                ))
                .child(choice(s, "pf-sign-ssh", "SSH", SigningKind::Ssh))
                .child(choice(s, "pf-sign-gpg", "GPG", SigningKind::Gpg)),
        ));
    if s.form.signing != SigningKind::None {
        let (caption, placeholder) = if s.form.signing == SigningKind::Ssh {
            (
                "Clé publique SSH (chemin du fichier .pub)",
                "/home/vous/.ssh/id_ed25519.pub",
            )
        } else {
            ("Identifiant de clé GPG (empreinte)", "0123456789ABCDEF…")
        };
        form = form
            .child(labelled(
                s,
                caption,
                field(s, "pf-key", caption, &s.form.key, placeholder)
                    .on_edit(|s: &mut State| &mut s.form.key),
            ))
            .child(
                text("GitPoulpe ne stocke que cette référence : jamais de clé privée ni de phrase secrète."
                    .to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
            );
        if let Some(status) = signing_status(s) {
            form = form.child(status);
        }
    }
    if let Some(error) = &s.form.error {
        form = form.child(text(error.clone()).font(theme::BODY).color(p.red).wrap());
    }
    let mut actions = row().gap(8.0).child(
        button(s, "pf-save".to_owned(), "Enregistrer".to_owned(), true)
            .on_press(|s: &mut State| s.save_profile()),
    );
    if s.form.editing.is_some() {
        actions = actions.child(
            button(
                s,
                "pf-delete".to_owned(),
                "Supprimer le profil".to_owned(),
                false,
            )
            .on_press(|s: &mut State| s.delete_profile()),
        );
    }
    form = form.child(actions);
    if let (Some(repo), Some(editing)) = (&s.repo, s.form.editing.clone()) {
        let active = s.active_profile().map(|a| a.id.clone());
        let mut here = col().gap(8.0).child(
            text(format!(
                "Dépôt ouvert : {}",
                display_label(std::path::Path::new(&repo.name))
            ))
            .font(theme::SMALL)
            .color(p.muted),
        );
        let mut buttons = row().gap(8.0);
        if active.as_deref() != Some(editing.as_str()) {
            let id = editing.clone();
            buttons = buttons.child(
                button(
                    s,
                    "pf-use".to_owned(),
                    "Utiliser ce profil pour ce dépôt".to_owned(),
                    false,
                )
                .on_press(move |s: &mut State| s.use_profile_here(Some(id.clone()))),
            );
        }
        if active.is_some() {
            buttons = buttons.child(
                button(
                    s,
                    "pf-unuse".to_owned(),
                    "Suivre la configuration Git".to_owned(),
                    false,
                )
                .on_press(|s: &mut State| s.use_profile_here(None)),
            );
        }
        here = here.child(buttons);
        form = form.child(here);
    }
    if s.repo.is_some() {
        form = form.child(identity_box(s));
    }
    let body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .py(16.0)
        .gap(16.0)
        .child(
            text("Un profil choisit le nom, l'e-mail et la signature utilisés par GitPoulpe pour un dépôt. Rien n'est écrit dans la configuration Git : le profil ne s'applique qu'aux commandes lancées par GitPoulpe."
                .to_owned())
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(row().w_full().gap(24.0).child(list).child(form));
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(crate::widgets::screen_head(
            s,
            "pf-head",
            "Profils Git",
            String::new(),
            vec![
                button(s, "pf-close".to_owned(), "Fermer".to_owned(), false)
                    .on_press(|s: &mut State| s.close_profiles()),
            ],
        ))
        .child(body)
}
