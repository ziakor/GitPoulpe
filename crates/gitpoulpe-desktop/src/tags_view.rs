use std::borrow::Cow;

use rune::{El, GrowthSpec, Key, ScrollSpec, col, el, row, text, text_input};

use crate::history_state::format_date;
use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 44.0;

fn short(oid: &str) -> String {
    oid.chars().take(8).collect()
}

fn action(s: &State, key: &str, caption: &str, primary: bool) -> El {
    let p = s.palette;
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key(key.to_owned())
        .role("button")
        .label(caption.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .min_h(32.0)
        .px(12.0)
        .py(6.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .child(text(caption.to_owned()).font(theme::BODY).color(ink).wrap())
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
        .on_input_key(|_: &mut State, event, tx| {
            if event.key == Key::Escape {
                tx.blur();
                return true;
            }
            false
        })
}

fn tag_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(tag) = s.tags.tags.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = s.tags.selected.as_deref() == Some(tag.name.as_str());
    let name = tag.name.clone();
    let kind = if tag.annotated { "annoté" } else { "léger" };
    let target = tag
        .target
        .as_deref()
        .map_or_else(|| "pas un commit".to_owned(), short);
    let detail = if tag.subject.is_empty() {
        format!("{kind} · {target}")
    } else {
        format!("{kind} · {target} · {}", tag.subject)
    };
    el().key(format!("tg-row{index}"))
        .role("option")
        .label(tag.name.clone())
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_tag(&name))
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(tag.name.clone())
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                )
                .child(text(detail).font(theme::SMALL).color(p.muted).truncate()),
        )
}

fn selected_detail(s: &State, mut panel: El) -> El {
    let p = s.palette;
    let Some(tag) = s.tags.selected_tag() else {
        return panel.child(
            text("Sélectionne un tag pour voir ses détails.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    panel = panel.child(el().h(1.0).w_full().fill(p.line)).child(
        text(tag.name.clone())
            .key("tag-selected")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .wrap()
            .selectable(true),
    );
    let kind = if tag.annotated {
        "Tag annoté"
    } else {
        "Tag léger"
    };
    panel = panel.child(
        text(format!(
            "{kind} · commit {}",
            tag.target.as_deref().map_or_else(
                || "aucun (pointe sur un autre objet)".to_owned(),
                str::to_owned
            )
        ))
        .key("tag-kind")
        .font(theme::PATH_SMALL)
        .color(p.muted)
        .wrap()
        .selectable(true),
    );
    if let (Some(tagger), Some(email)) = (&tag.tagger, &tag.tagger_email) {
        let when = tag
            .timestamp
            .map(|t| format!(" · {} UTC", format_date(t)))
            .unwrap_or_default();
        panel = panel.child(
            text(format!("{tagger} <{email}>{when}"))
                .key("tag-tagger")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    if let Some(message) = &tag.message {
        panel = panel.child(
            text(message.clone())
                .key("tag-message")
                .font(theme::BODY)
                .color(p.text)
                .wrap()
                .selectable(true),
        );
    }
    if let Some(commit) = tag.target.clone() {
        panel = panel.child(
            action(s, "tag-inspect", "Inspecter le commit", false)
                .on_press(move |s: &mut State| s.open_commit(commit.clone(), 0)),
        );
    }
    if s.tags.confirm_delete.as_deref() == Some(tag.name.as_str()) {
        panel = panel.child(
            col()
                .key("tag-confirm")
                .w_full()
                .gap(6.0)
                .px(12.0)
                .py(10.0)
                .radius(3.0)
                .border(1.0, p.red)
                .child(
                    text(format!(
                        "Supprimer le tag « {} » ? Aucune branche n'est touchée. Pour le rétablir : git update-ref refs/tags/{} {}",
                        tag.name, tag.name, tag.object
                    ))
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            action(s, "tag-delete-confirm", "Supprimer le tag", true)
                                .on_press(|s: &mut State| s.tag_delete()),
                        )
                        .child(
                            action(s, "tag-delete-cancel", "Annuler", false)
                                .on_press(|s: &mut State| s.tag_cancel_delete()),
                        ),
                ),
        );
    } else {
        panel = panel.child(
            action(s, "tag-delete", "Supprimer ce tag", false)
                .on_press(|s: &mut State| s.tag_ask_delete()),
        );
    }
    panel
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let target = s.tags.target.as_deref().map_or_else(
        || {
            s.repo
                .as_ref()
                .and_then(|r| r.snapshot.branch.oid.as_deref().map(short))
                .map_or_else(|| "HEAD".to_owned(), |oid| format!("HEAD ({oid})"))
        },
        |oid| format!("le commit {}", short(oid)),
    );
    let panel = col()
        .w(400.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            text(format!("Nouveau tag sur {target}"))
                .key("tag-target")
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            field(s, "tag-new", "Nom du tag", &s.tags.new_name, "v1.0.0")
                .on_edit(|s: &mut State| &mut s.tags.new_name),
        )
        .child(
            field(
                s,
                "tag-message-input",
                "Message du tag annoté",
                &s.tags.message,
                "Message (tag annoté)",
            )
            .on_edit(|s: &mut State| &mut s.tags.message),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    action(s, "tag-create-light", "Tag léger", false)
                        .on_press(|s: &mut State| s.tag_create(false)),
                )
                .child(
                    action(s, "tag-create-annotated", "Tag annoté", true)
                        .on_press(|s: &mut State| s.tag_create(true)),
                ),
        );
    selected_detail(s, panel)
}

pub fn tags_screen(s: &State) -> El {
    let p = s.palette;
    let title = if s.tags.loading {
        "Tags · lecture…".to_owned()
    } else {
        format!("Tags · {}", s.tags.tags.len())
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
            action(s, "tags-branches", "Branches", false)
                .on_press(|s: &mut State| s.open_branches()),
        )
        .child(action(s, "tags-close", "Fermer", false).on_press(|s: &mut State| s.close_tags()));
    let list: El = if let Some(error) = &s.tags.error {
        col().w_full().grow(1.0).center().child(
            text(format!("Lecture impossible : {error}"))
                .font(theme::BODY)
                .color(p.red)
                .wrap(),
        )
    } else if s.tags.tags.is_empty() && !s.tags.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun tag dans ce dépôt.".to_owned())
                .key("tags-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("tags-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("tags-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.tags.version, s.dark))
                    .virtual_rows(
                        s.tags.tags.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("tk{i}")),
                        tag_row,
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
