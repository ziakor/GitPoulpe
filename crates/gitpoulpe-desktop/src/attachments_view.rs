use gitpoulpe_hosting::{Attachment, AttachmentKind, PullFile};
use rune::{Color, El, Image, TextStyle, col, el, row, text};

use crate::attachments::Media;
use crate::state::State;
use crate::theme;
use crate::widgets::action;

const IMAGE_MAX_W: f32 = 720.0;
const IMAGE_MAX_H: f32 = 480.0;

pub struct Body<'a> {
    pub key: String,
    pub prefix: String,
    pub text: &'a str,
    pub suffix: String,
    pub attachments: &'a [Attachment],
    pub style: TextStyle,
    pub color: Color,
}

enum Piece<'a> {
    Text(&'a str),
    Attachment(&'a Attachment),
}

fn pieces<'a>(body: &'a str, attachments: &'a [Attachment]) -> Vec<Piece<'a>> {
    let mut sorted: Vec<&Attachment> = attachments
        .iter()
        .filter(|a| {
            a.span.start <= a.span.end
                && body.is_char_boundary(a.span.start)
                && body.is_char_boundary(a.span.end)
        })
        .collect();
    sorted.sort_by_key(|a| a.span.start);
    let mut out = Vec::new();
    let mut at = 0;
    for attachment in sorted {
        if attachment.span.start < at {
            continue;
        }
        out.push(Piece::Text(&body[at..attachment.span.start]));
        out.push(Piece::Attachment(attachment));
        at = attachment.span.end;
    }
    out.push(Piece::Text(&body[at..]));
    out
}

pub fn rich_body(s: &State, body: Body<'_>) -> El {
    let line = |content: String, key: String| {
        text(content)
            .key(key)
            .font(body.style)
            .color(body.color)
            .wrap()
    };
    if body.attachments.is_empty() {
        return line(
            format!("{}{}{}", body.prefix, body.text, body.suffix),
            body.key.clone(),
        );
    }
    let parts = pieces(body.text, body.attachments);
    let shown = |piece: &Piece| match piece {
        Piece::Text(t) => !t.trim().is_empty(),
        Piece::Attachment(_) => false,
    };
    let last_text = parts.iter().rposition(shown);
    let mut prefix = Some(body.prefix.clone());
    let mut column = col().key(body.key.clone()).w_full().gap(6.0);
    for (i, piece) in parts.iter().enumerate() {
        match piece {
            Piece::Text(_) if !shown(piece) => {}
            Piece::Text(t) => {
                let suffix = if last_text == Some(i) {
                    body.suffix.as_str()
                } else {
                    ""
                };
                let content = t.trim_matches(['\n', '\r']).trim_end();
                let lead = prefix.take().unwrap_or_default();
                column = column.child(line(
                    format!("{lead}{content}{suffix}"),
                    format!("{}-t{i}", body.key),
                ));
            }
            Piece::Attachment(attachment) => {
                if let Some(lead) = prefix.take().filter(|l| !l.trim().is_empty()) {
                    column = column.child(line(
                        lead.trim_end().to_owned(),
                        format!("{}-lead", body.key),
                    ));
                }
                column = column.child(attachment_el(s, &format!("{}-a{i}", body.key), attachment));
            }
        }
    }
    if last_text.is_none() && !body.suffix.trim().is_empty() {
        column = column.child(line(
            body.suffix.trim_start().to_owned(),
            format!("{}-end", body.key),
        ));
    }
    column
}

struct Shown<'a> {
    key: &'a str,
    media: &'a str,
    fetched: bool,
    name: &'a str,
    link: &'a str,
}

fn image_slot(s: &State, shown: Shown<'_>) -> El {
    let name = shown.name;
    let label = match s.pulls.list.media.get(shown.media) {
        Some(Media::Ready(image, size)) => return picture(s, &shown, image, *size),
        Some(Media::Failed(why)) => format!("Image « {name} » non affichée : {why}."),
        Some(Media::Loading | Media::Decoding(_)) => format!("Chargement de l'image « {name} »…"),
        None if shown.fetched => format!("Chargement de l'image « {name} »…"),
        None => format!("Image « {name} » non chargée : elle est hébergée hors du fournisseur."),
    };
    card(s, shown.key, label, shown.link)
}

fn attachment_el(s: &State, key: &str, attachment: &Attachment) -> El {
    let name = &attachment.name;
    let label = match attachment.kind {
        AttachmentKind::Image => {
            return image_slot(
                s,
                Shown {
                    key,
                    media: &attachment.link,
                    fetched: attachment.preview.is_some(),
                    name,
                    link: &attachment.link,
                },
            );
        }
        AttachmentKind::Video => format!("Vidéo « {name} »"),
        AttachmentKind::File => format!("Fichier joint « {name} »"),
    };
    card(s, key, label, &attachment.link)
}

pub fn binary_file(s: &State, index: usize, file: &PullFile) -> El {
    let p = s.palette;
    let key = format!("prl-file{index}-bin");
    let note = "Pas de diff texte fourni pour ce fichier (binaire ou trop volumineux).";
    let Some(media) = &file.media else {
        return text(note.to_owned())
            .key(key)
            .font(theme::SMALL)
            .color(p.muted)
            .wrap();
    };
    if media.before.is_none() && media.after.is_none() {
        return card(s, &key, note.to_owned(), &media.link);
    }
    let status = file.change.status.as_str();
    let side =
        |title: &str, which: &str, preview: Option<&gitpoulpe_hosting::Preview>, missing: &str| {
            let slot_key = format!("prl-file{index}-{which}");
            let body = match preview {
                Some(preview) => image_slot(
                    s,
                    Shown {
                        key: &slot_key,
                        media: preview.key(),
                        fetched: true,
                        name: &file.change.path,
                        link: &media.link,
                    },
                ),
                None => text(missing.to_owned())
                    .key(slot_key.clone())
                    .font(theme::SMALL)
                    .color(p.muted)
                    .wrap(),
            };
            col()
                .w_full()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text(title.to_owned()).font(theme::SMALL).color(p.muted))
                .child(body)
        };
    let before_missing = if status == "added" {
        "Nouveau fichier."
    } else {
        "Version précédente indisponible."
    };
    row()
        .key(key)
        .w_full()
        .gap(12.0)
        .child(side(
            "Avant",
            "before",
            media.before.as_ref(),
            before_missing,
        ))
        .child(side(
            "Après",
            "after",
            media.after.as_ref(),
            "Fichier supprimé.",
        ))
}

fn card(s: &State, key: &str, label: String, link: &str) -> El {
    let p = s.palette;
    let url = link.to_owned();
    row()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .px(10.0)
        .py(6.0)
        .center_y()
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .child(
            text(label)
                .font(theme::SMALL)
                .color(p.text)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            action(
                s,
                &format!("{key}-open"),
                "Ouvrir dans le navigateur",
                false,
            )
            .on_press_tx(move |s: &mut State, tx| s.open_link(tx, &url)),
        )
}

fn picture(s: &State, shown: &Shown<'_>, image: &Image, size: [u32; 2]) -> El {
    let p = s.palette;
    let key = shown.key;
    let [w, h] = size.map(|v| v.max(1) as f32);
    let ratio = w / h;
    let width = w.min(IMAGE_MAX_W).min(IMAGE_MAX_H * ratio);
    let url = shown.link.to_owned();
    let mut column = col().key(key.to_owned()).w_full().gap(2.0).child(
        el().key(format!("{key}-image"))
            .role("image")
            .label(shown.name.to_owned())
            .w_full()
            .max_w(width)
            .aspect_ratio(ratio)
            .pixels(image.pixels())
            .radius(3.0)
            .border(1.0, p.line)
            .on_press_tx(move |s: &mut State, tx| s.open_link(tx, &url)),
    );
    if !shown.name.trim().is_empty() && !shown.name.eq_ignore_ascii_case("image") {
        column = column.child(
            text(shown.name.to_owned())
                .key(format!("{key}-caption"))
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    column
}
