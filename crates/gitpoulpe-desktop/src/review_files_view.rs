use std::borrow::Cow;
use std::rc::Rc;

use gitpoulpe_hosting::{PullDetail, PullFile, ReviewComment, Side};
use rune::{Align, El, GrowthSpec, col, el, row, text};

use crate::attachments_view::{Body, binary_file, rich_body};
use crate::pull_list::{CommentKind, NumberedLine, number_files, patch_size, when};
use crate::state::State;
use crate::theme;
use crate::widgets::{action, area, quiet, toggle};

const LINE: f32 = 22.0;
const NUMBER: f32 = 44.0;

fn status_word(status: &str) -> &'static str {
    match status {
        "added" => "Ajouté",
        "removed" => "Supprimé",
        "renamed" => "Renommé",
        "copied" => "Copié",
        _ => "Modifié",
    }
}

pub fn send_status(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut block = col().w_full().gap(4.0);
    if list.sending {
        block = block.child(
            text("Envoi…".to_owned())
                .key("prc-sending")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    if let Some(notice) = &list.send_notice {
        block = block.child(
            text(notice.clone())
                .key("prc-notice")
                .font(theme::SMALL)
                .color(p.text)
                .w_full()
                .wrap(),
        );
    }
    if let Some(error) = &list.send_error {
        block = block.child(
            text(error.clone())
                .key("prc-error")
                .font(theme::SMALL)
                .color(p.red)
                .w_full()
                .wrap(),
        );
    }
    if list.unverified.is_some() {
        block = block
            .child(
                text("Le résultat de l'envoi est inconnu : vérifiez avant de réessayer, pour ne pas publier deux fois.".to_owned())
                    .key("prc-unknown")
                    .font(theme::SMALL)
                    .color(p.red)
                    .w_full()
                    .wrap(),
            )
            .child(
                action(s, "prc-verify", "Vérifier si le commentaire est parti", false)
                    .on_press(|s: &mut State| s.pull_verify_comment()),
            );
    }
    block
}

fn line_composer(s: &State) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let target = match (&list.reply_to, &list.pick) {
        (Some(_), _) => "Réponse au fil".to_owned(),
        (None, Some(k)) => format!(
            "Commentaire sur la ligne {}{} ({}) de {}",
            k.start.map(|s| format!("{s}–")).unwrap_or_default(),
            k.line,
            if k.side == Side::Left {
                "ancien côté"
            } else {
                "nouveau côté"
            },
            k.path
        ),
        (None, None) => String::new(),
    };
    let mut block = col()
        .key("prc-box")
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(8.0)
        .fill(p.panel)
        .border_sides([0.0, 0.0, 0.0, 2.0], p.accent)
        .child(
            text(target)
                .key("prc-target")
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .wrap(),
        );
    if list.reply_to.is_none() {
        block = block.child(
            toggle(
                s,
                "prc-range",
                "Étendre à une plage (cliquez ensuite la dernière ligne)",
                list.range_mode,
            )
            .on_press(|s: &mut State| s.pull_toggle_range()),
        );
    }
    block = block.child(
        area(
            s,
            "prc-body",
            "Commentaire",
            &list.compose,
            "Votre commentaire",
        )
        .on_edit(|s: &mut State| &mut s.pulls.list.compose),
    );
    let mut buttons = row().gap(8.0).center_y();
    if list.reply_to.is_some() {
        buttons = buttons.child(
            action(s, "prc-send-reply", "Répondre", true)
                .on_press(|s: &mut State| s.pull_send_comment(CommentKind::Reply)),
        );
    } else {
        buttons = buttons
            .child(
                action(s, "prc-add-draft", "Ajouter à la revue", true)
                    .on_press(|s: &mut State| s.pull_add_to_draft()),
            )
            .child(
                action(s, "prc-send-line", "Commenter maintenant", false)
                    .on_press(|s: &mut State| s.pull_send_comment(CommentKind::Line)),
            );
    }
    buttons = buttons
        .child(quiet(s, "prc-cancel", "Annuler").on_press(|s: &mut State| s.pull_clear_target()));
    block.child(buttons).child(send_status(s))
}

fn comment_block(s: &State, key: String, comment: &ReviewComment, note: String) -> El {
    let p = s.palette;
    let mut head = row().w_full().gap(6.0).center_y().child(
        text(comment.author.clone())
            .font(theme::BODY_STRONG)
            .color(p.text),
    );
    if !note.is_empty() {
        head = head.child(text(note).font(theme::SMALL).color(p.muted));
    }
    head = head.child(
        text(format!("· {}", when(&comment.created_at)))
            .font(theme::SMALL)
            .color(p.muted),
    );
    col().w_full().gap(4.0).child(head).child(rich_body(
        s,
        Body {
            key,
            prefix: String::new(),
            text: &comment.body,
            suffix: String::new(),
            attachments: &comment.attachments,
            style: theme::BODY,
            color: p.text,
        },
    ))
}

fn thread(s: &State, file_index: usize, root: &ReviewComment) -> El {
    let p = s.palette;
    let comments = &s.pulls.list.comments;
    let id = root.id;
    let place = format!(
        "ligne {}{}",
        root.line
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_owned()),
        if root.outdated {
            ", version précédente"
        } else {
            ""
        }
    );
    let mut block = col()
        .key(format!("prc-thread{id}"))
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(10.0)
        .fill(p.panel)
        .border_sides([0.0, 0.0, 0.0, 2.0], p.line)
        .child(comment_block(
            s,
            format!("prc-f{file_index}c{id}"),
            root,
            place,
        ));
    for reply in comments
        .review
        .iter()
        .filter(|c| c.in_reply_to == Some(root.id))
    {
        block = block.child(comment_block(
            s,
            format!("prc-f{file_index}r{}", reply.id),
            reply,
            String::new(),
        ));
    }
    if s.pulls.list.reply_to == Some(id) {
        block = block.child(line_composer(s));
    } else {
        block = block.child(
            quiet(s, &format!("prc-reply{id}"), "Répondre")
                .on_press(move |s: &mut State| s.pull_reply_to(id)),
        );
    }
    block
}

fn number(value: Option<u64>, ink: rune::Color) -> El {
    text(value.map(|n| n.to_string()).unwrap_or_default())
        .font(theme::PATH_SMALL)
        .color(ink)
        .w(NUMBER)
        .align(Align::End)
}

fn diff_line(s: &State, path: &str, file_index: usize, n: usize, line: &NumberedLine) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let mut chars = line.text.chars();
    let first = chars.next();
    let rest = chars.as_str();
    let (fill, sign, ink, content) = match first {
        Some('@') => (Some(p.panel), " ", p.muted, line.text.as_str()),
        Some('+') => (Some(p.added), "+", p.green, rest),
        Some('-') => (Some(p.removed), "−", p.red, rest),
        Some('\\') => (None, " ", p.muted, line.text.as_str()),
        Some(_) => (None, " ", p.muted, rest),
        None => (None, " ", p.muted, ""),
    };
    let picked = list.pick.as_ref().is_some_and(|k| {
        k.path == path
            && line.anchor.is_some_and(|(l, side)| {
                side == k.side && l >= k.start.unwrap_or(k.line) && l <= k.line
            })
    });
    let rule = if picked {
        el().w(2.0).h_full().fill(p.accent)
    } else {
        el().w(2.0).h_full()
    };
    let mut el_line = row()
        .key(format!("prl-f{file_index}l{n}"))
        .w_full()
        .h(LINE)
        .center_y()
        .child(rule)
        .child(number(line.old, p.muted))
        .child(number(line.new, p.muted))
        .child(
            text(sign.to_owned())
                .font(theme::PATH_SMALL)
                .color(ink)
                .w(22.0)
                .align(Align::Center),
        )
        .child(
            text(content.to_owned())
                .font(theme::PATH_SMALL)
                .color(if line.text.starts_with('@') {
                    p.muted
                } else {
                    p.text
                })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if let Some(fill) = fill {
        el_line = el_line.fill(fill);
    }
    if picked {
        el_line = el_line.fill(p.selected);
    }
    if let Some((number, side)) = line.anchor {
        let path = path.to_owned();
        el_line = el_line
            .role("button")
            .label(format!("Commenter la ligne {number}"))
            .hover(p.hover)
            .on_press(move |s: &mut State| s.pull_pick_line(path.clone(), number, side));
    }
    el_line
}

#[derive(Clone, Copy)]
enum FileRow {
    Summary,
    Header(usize),
    Line(usize, usize),
    Thread(usize, u64),
    Composer,
    Binary(usize),
    Others(usize),
    Gap(usize),
}

fn row_key(row: FileRow) -> String {
    match row {
        FileRow::Summary => "prl-files-title".to_owned(),
        FileRow::Header(i) => format!("prl-fh{i}"),
        FileRow::Line(i, n) => format!("prl-f{i}l{n}"),
        FileRow::Thread(_, id) => format!("prc-thread{id}"),
        FileRow::Composer => "prc-box".to_owned(),
        FileRow::Binary(i) => format!("prl-fb{i}"),
        FileRow::Others(i) => format!("prl-fo{i}"),
        FileRow::Gap(i) => format!("prl-fg{i}"),
    }
}

fn plan(s: &State, detail: &PullDetail, numbered: &[Vec<NumberedLine>]) -> Vec<FileRow> {
    let list = &s.pulls.list;
    let mut rows = vec![FileRow::Summary];
    let mut placed = false;
    for (i, file) in detail.files.iter().enumerate() {
        rows.push(FileRow::Header(i));
        let roots: Vec<&ReviewComment> = list
            .comments
            .review
            .iter()
            .filter(|c| c.path == file.change.path && c.in_reply_to.is_none())
            .collect();
        let mut shown: Vec<u64> = Vec::new();
        match (&file.patch, numbered.get(i)) {
            (Some(_), Some(lines)) => {
                for (n, line) in lines.iter().enumerate() {
                    rows.push(FileRow::Line(i, n));
                    let Some((number, side)) = line.anchor else {
                        continue;
                    };
                    for root in roots
                        .iter()
                        .filter(|r| !r.outdated && r.line == Some(number) && r.side == side)
                    {
                        shown.push(root.id);
                        placed |= list.reply_to == Some(root.id);
                        rows.push(FileRow::Thread(i, root.id));
                    }
                    let here = list.pick.as_ref().is_some_and(|k| {
                        k.path == file.change.path && k.line == number && k.side == side
                    });
                    if here && list.reply_to.is_none() && !placed {
                        placed = true;
                        rows.push(FileRow::Composer);
                    }
                }
            }
            _ => rows.push(FileRow::Binary(i)),
        }
        let rest: Vec<u64> = roots
            .iter()
            .filter(|r| !shown.contains(&r.id))
            .map(|r| r.id)
            .collect();
        if !rest.is_empty() {
            rows.push(FileRow::Others(i));
            for id in rest {
                placed |= list.reply_to == Some(id);
                rows.push(FileRow::Thread(i, id));
            }
        }
        rows.push(FileRow::Gap(i));
    }
    if !placed && (list.pick.is_some() || list.reply_to.is_some()) {
        rows.push(FileRow::Composer);
    }
    rows
}

fn file_header(s: &State, index: usize, file: &PullFile) -> El {
    let p = s.palette;
    let change = &file.change;
    row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .fill(p.raised)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            text(change.path.clone())
                .key(format!("prl-file{index}"))
                .font(theme::PATH)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            text(status_word(&change.status).to_owned())
                .font(theme::TINY)
                .color(p.muted),
        )
        .child(
            text(format!("+{}", change.additions))
                .font(theme::TINY)
                .color(p.green),
        )
        .child(
            text(format!("−{}", change.deletions))
                .font(theme::TINY)
                .color(p.red),
        )
}

fn summary(s: &State, detail: &PullDetail) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let (added, removed) = detail.files.iter().fold((0, 0), |(a, r), f| {
        (a + f.change.additions, r + f.change.deletions)
    });
    let mut block = col().w_full().gap(6.0).pt(8.0).pb(12.0).child(
        text(format!(
            "{} fichier{}{} · +{added} −{removed}",
            detail.files.len(),
            if detail.files.len() > 1 { "s" } else { "" },
            if detail.files_truncated {
                " (liste tronquée)"
            } else {
                ""
            }
        ))
        .font(theme::SMALL)
        .color(p.muted),
    );
    if list.pick.is_none() && list.reply_to.is_none() {
        block = block.child(send_status(s));
    }
    block
}

fn build_row(s: &State, row: FileRow, numbered: &[Vec<NumberedLine>]) -> El {
    let p = s.palette;
    let Some(detail) = s.pulls.list.detail.as_ref() else {
        return el();
    };
    let file = |i: usize| detail.files.get(i);
    match row {
        FileRow::Summary => summary(s, detail),
        FileRow::Header(i) => file(i).map_or_else(el, |f| file_header(s, i, f)),
        FileRow::Line(i, n) => match (file(i), numbered.get(i).and_then(|l| l.get(n))) {
            (Some(f), Some(line)) => diff_line(s, &f.change.path, i, n, line),
            _ => el(),
        },
        FileRow::Thread(i, id) => s
            .pulls
            .list
            .comments
            .review
            .iter()
            .find(|c| c.id == id)
            .map_or_else(el, |root| thread(s, i, root)),
        FileRow::Composer => line_composer(s),
        FileRow::Binary(i) => {
            file(i).map_or_else(el, |f| col().w_full().py(8.0).child(binary_file(s, i, f)))
        }
        FileRow::Others(_) => col().w_full().pt(10.0).pb(4.0).child(
            text("Commentaires hors des lignes du diff".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        ),
        FileRow::Gap(_) => el().h(16.0),
    }
}

pub fn files_tab(s: &State, detail: &PullDetail) -> El {
    let list = &s.pulls.list;
    let numbered = match &list.numbered {
        Some(cached)
            if cached.number == detail.summary.number
                && cached.head == detail.summary.head_sha
                && cached.files.len() == detail.files.len()
                && cached.size == patch_size(detail) =>
        {
            cached.files.clone()
        }
        _ => number_files(detail),
    };
    let rows = Rc::new(plan(s, detail, &numbered));
    let keys = rows.clone();
    col()
        .key("prl-files")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((list.version, s.dark, rows.len()))
        .virtual_rows(
            rows.len(),
            LINE,
            move |_: &State, i| Cow::Owned(row_key(keys[i])),
            move |s: &State, i| build_row(s, rows[i], &numbered),
        )
}
