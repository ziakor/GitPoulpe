use std::borrow::Cow;
use std::rc::Rc;

use gitpoulpe_hosting::{PullDetail, PullFile, ReviewComment, Side};
use rune::{Align, El, GrowthSpec, Span, col, el, row, text};

use crate::attachments_view::{Body, binary_file, rich_body};
use crate::pull_list::{CommentKind, NumberedLine, current_numbered, when};
use crate::state::{DiffMode, State};
use crate::theme::{self, Palette};
use crate::view::mode_button;
use crate::widgets::{action, area, chevron_icon, quiet, toggle};

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
            "Commentaire sur {} ({}) de {}",
            match k.start {
                Some(start) => format!("les lignes {start} à {}", k.line),
                None => format!("la ligne {}", k.line),
            },
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
    if list.reply_to.is_none()
        && list.pick.as_ref().is_some_and(|k| k.start.is_none())
        && !s.pull_is_gitlab()
    {
        block = block.child(
            text("Plusieurs lignes : glissez de la première à la dernière, ou Maj+clic sur la dernière.".to_owned())
                .key("prc-range-hint")
                .font(theme::SMALL)
                .color(p.muted)
                .w_full()
                .wrap(),
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

struct Look<'a> {
    fill: Option<rune::Color>,
    sign: &'static str,
    ink: rune::Color,
    content: &'a str,
}

fn look<'a>(p: &Palette, line: &'a NumberedLine) -> Look<'a> {
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
    Look {
        fill,
        sign,
        ink,
        content,
    }
}

fn is_context(line: &NumberedLine) -> bool {
    !matches!(
        line.text.as_bytes().first(),
        Some(b'@' | b'+' | b'-' | b'\\')
    )
}

fn code(p: &Palette, line: &NumberedLine, content: &str) -> El {
    text(content.to_owned())
        .font(theme::PATH_SMALL)
        .color(if line.text.starts_with('@') {
            p.muted
        } else {
            p.text
        })
        .spans(
            line.marks
                .iter()
                .filter_map(|(range, paint)| {
                    let mut span = Span::new(range.start, range.end);
                    span.color = Some(p.syntax(*paint)?);
                    Some(span)
                })
                .collect::<Vec<_>>(),
        )
        .grow(1.0)
        .min_w(0.0)
        .truncate()
}

fn sign(look: &Look) -> El {
    text(look.sign.to_owned())
        .font(theme::PATH_SMALL)
        .color(look.ink)
        .w(22.0)
        .align(Align::Center)
}

fn picked(s: &State, path: &str, line: &NumberedLine) -> bool {
    let list = &s.pulls.list;
    let chosen = match (&list.line_drag, &list.pick) {
        (Some(d), _) => Some((
            d.path.as_str(),
            d.side,
            d.anchor.min(d.end),
            d.anchor.max(d.end),
        )),
        (None, Some(k)) => Some((k.path.as_str(), k.side, k.start.unwrap_or(k.line), k.line)),
        (None, None) => None,
    };
    chosen.is_some_and(|(at, chosen_side, first, last)| {
        at == path
            && line
                .anchor
                .is_some_and(|(l, side)| side == chosen_side && (first..=last).contains(&l))
    })
}

fn rule(p: &Palette, picked: bool) -> El {
    let rule = el().w(2.0).h_full();
    if picked { rule.fill(p.accent) } else { rule }
}

fn shade(row: El, p: &Palette, fill: Option<rune::Color>, picked: bool) -> El {
    match (picked, fill) {
        (true, _) => row.fill(p.selected),
        (false, Some(fill)) => row.fill(fill),
        (false, None) => row,
    }
}

fn commentable(row: El, s: &State, path: &str, line: &NumberedLine) -> El {
    let Some((number, side)) = line.anchor else {
        return row;
    };
    let (pressed, dragged) = (path.to_owned(), path.to_owned());
    row.role("button")
        .label(format!("Commenter la ligne {number}"))
        .hover(s.palette.hover)
        .on_press_tx(move |s: &mut State, tx| {
            s.pull_line_pressed(pressed.clone(), number, side, tx.mods().shift)
        })
        .on_drag(move |s: &mut State, drag, tx| s.pull_line_drag(&dragged, number, side, drag, tx))
}

fn diff_line(s: &State, path: &str, file_index: usize, n: usize, line: &NumberedLine) -> El {
    let p = s.palette;
    let look = look(&p, line);
    let picked = picked(s, path, line);
    let row = row()
        .key(format!("prl-f{file_index}l{n}"))
        .w_full()
        .h(LINE)
        .center_y()
        .child(rule(&p, picked))
        .child(number(line.old, p.muted))
        .child(number(line.new, p.muted))
        .child(sign(&look))
        .child(code(&p, line, look.content));
    commentable(shade(row, &p, look.fill, picked), s, path, line)
}

fn half() -> El {
    row().w(0.0).grow(1.0).min_w(0.0).h_full().center_y()
}

fn split_context(s: &State, path: &str, file_index: usize, n: usize, line: &NumberedLine) -> El {
    let p = s.palette;
    let look = look(&p, line);
    let picked = picked(s, path, line);
    let side = |value: Option<u64>| {
        half()
            .child(number(value, p.muted))
            .child(sign(&look))
            .child(code(&p, line, look.content))
    };
    let row = row()
        .key(format!("prl-f{file_index}l{n}"))
        .w_full()
        .h(LINE)
        .center_y()
        .child(rule(&p, picked))
        .child(side(line.old))
        .child(el().w(1.0).h_full().fill(p.line))
        .child(side(line.new));
    commentable(shade(row, &p, None, picked), s, path, line)
}

fn split_cell(
    s: &State,
    path: &str,
    file_index: usize,
    at: Option<usize>,
    lines: &[NumberedLine],
) -> El {
    let p = s.palette;
    let Some((n, line)) = at.and_then(|n| lines.get(n).map(|line| (n, line))) else {
        return half().fill(p.panel);
    };
    let look = look(&p, line);
    let picked = picked(s, path, line);
    let cell = half()
        .key(format!("prl-f{file_index}l{n}"))
        .child(rule(&p, picked))
        .child(number(line.old.or(line.new), p.muted))
        .child(sign(&look))
        .child(code(&p, line, look.content));
    commentable(shade(cell, &p, look.fill, picked), s, path, line)
}

fn split_change(
    s: &State,
    path: &str,
    file_index: usize,
    (left, right): (Option<usize>, Option<usize>),
    lines: &[NumberedLine],
) -> El {
    let p = s.palette;
    row()
        .key(change_key(file_index, left, right))
        .w_full()
        .h(LINE)
        .child(split_cell(s, path, file_index, left, lines))
        .child(el().w(1.0).h_full().fill(p.line))
        .child(split_cell(s, path, file_index, right, lines))
}

fn change_key(file_index: usize, left: Option<usize>, right: Option<usize>) -> String {
    format!("prl-f{file_index}c{}", left.or(right).unwrap_or_default())
}

fn split_rows(file: usize, lines: &[NumberedLine]) -> Vec<FileRow> {
    let mut rows = Vec::with_capacity(lines.len());
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let flush = |rows: &mut Vec<FileRow>, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
        for k in 0..removed.len().max(added.len()) {
            rows.push(FileRow::Pair(
                file,
                removed.get(k).copied(),
                added.get(k).copied(),
            ));
        }
        removed.clear();
        added.clear();
    };
    for (n, line) in lines.iter().enumerate() {
        match line.text.as_bytes().first() {
            Some(b'-') => {
                if !added.is_empty() {
                    flush(&mut rows, &mut removed, &mut added);
                }
                removed.push(n);
            }
            Some(b'+') => added.push(n),
            _ => {
                flush(&mut rows, &mut removed, &mut added);
                rows.push(FileRow::Line(file, n));
            }
        }
    }
    flush(&mut rows, &mut removed, &mut added);
    rows
}

impl FileRow {
    fn lines(self) -> [Option<usize>; 2] {
        match self {
            FileRow::Line(_, n) => [Some(n), None],
            FileRow::Pair(_, left, right) => [left, right],
            _ => [None, None],
        }
    }
}

#[derive(Clone, Copy)]
enum FileRow {
    Summary,
    Header(usize),
    Line(usize, usize),
    Pair(usize, Option<usize>, Option<usize>),
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
        FileRow::Pair(i, left, right) => change_key(i, left, right),
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
        if s.pull_file_folded(&file.change.path) {
            let here = list
                .pick
                .as_ref()
                .is_some_and(|k| k.path == file.change.path);
            if here && list.reply_to.is_none() && !placed {
                placed = true;
                rows.push(FileRow::Composer);
            }
            rows.push(FileRow::Gap(i));
            continue;
        }
        let roots: Vec<&ReviewComment> = list
            .comments
            .review
            .iter()
            .filter(|c| c.path == file.change.path && c.in_reply_to.is_none())
            .collect();
        let mut shown: Vec<u64> = Vec::new();
        match (&file.patch, numbered.get(i)) {
            (Some(_), Some(lines)) => {
                let units = if s.mode == DiffMode::Split {
                    split_rows(i, lines)
                } else {
                    (0..lines.len()).map(|n| FileRow::Line(i, n)).collect()
                };
                for unit in units {
                    rows.push(unit);
                    for line in unit
                        .lines()
                        .into_iter()
                        .flatten()
                        .filter_map(|n| lines.get(n))
                    {
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
    let folded = s.pull_file_folded(&change.path);
    let viewed = s.pull_file_viewed(&change.path);
    let (fold_path, viewed_path) = (change.path.clone(), change.path.clone());
    row()
        .w_full()
        .pl(4.0)
        .pr(6.0)
        .py(4.0)
        .gap(10.0)
        .center_y()
        .fill(p.raised)
        .radius(3.0)
        .border(1.0, p.line)
        .child(
            row()
                .key(format!("prl-fold{index}"))
                .role("button")
                .label(format!(
                    "{} {}",
                    if folded { "Déplier" } else { "Replier" },
                    change.path
                ))
                .focusable(true)
                .focus_ring(2.0, 0.0, p.accent)
                .grow(1.0)
                .min_w(0.0)
                .px(6.0)
                .py(4.0)
                .gap(8.0)
                .center_y()
                .radius(3.0)
                .hover(p.hover)
                .on_press(move |s: &mut State| s.pull_toggle_fold(&fold_path))
                .child(chevron_icon(p.muted, !folded))
                .child(
                    text(change.path.clone())
                        .key(format!("prl-file{index}"))
                        .font(theme::PATH)
                        .color(if viewed { p.muted } else { p.text })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
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
        .child(
            toggle(s, &format!("prl-viewed{index}"), "Vu", viewed)
                .label(format!("Marquer {} comme vu", change.path))
                .on_press(move |s: &mut State| s.pull_toggle_viewed(&viewed_path)),
        )
}

fn summary(s: &State, detail: &PullDetail) -> El {
    let p = s.palette;
    let list = &s.pulls.list;
    let (added, removed) = detail.files.iter().fold((0, 0), |(a, r), f| {
        (a + f.change.additions, r + f.change.deletions)
    });
    let mut block = col().w_full().gap(6.0).pt(8.0).pb(12.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                text(format!(
                    "{} fichier{}{} · +{added} −{removed} · {} sur {} vu{}",
                    detail.files.len(),
                    if detail.files.len() > 1 { "s" } else { "" },
                    if detail.files_truncated {
                        " (liste tronquée)"
                    } else {
                        ""
                    },
                    s.pull_viewed_count(),
                    detail.files.len(),
                    if s.pull_viewed_count() > 1 { "s" } else { "" },
                ))
                .key("prl-files-summary")
                .font(theme::SMALL)
                .color(p.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
            )
            .child(
                row()
                    .gap(6.0)
                    .role("tablist")
                    .focus_group()
                    .child(mode_button(
                        s,
                        DiffMode::Unified,
                        "prl-mode-unified",
                        "Unifié",
                    ))
                    .child(mode_button(s, DiffMode::Split, "prl-mode-split", "Séparé")),
            ),
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
            (Some(f), Some(line)) if s.mode == DiffMode::Split && is_context(line) => {
                split_context(s, &f.change.path, i, n, line)
            }
            (Some(f), Some(line)) => diff_line(s, &f.change.path, i, n, line),
            _ => el(),
        },
        FileRow::Pair(i, left, right) => match (file(i), numbered.get(i)) {
            (Some(f), Some(lines)) => split_change(s, &f.change.path, i, (left, right), lines),
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
    let numbered = current_numbered(list.numbered.as_ref(), detail);
    let rows = Rc::new(plan(s, detail, &numbered));
    let keys = rows.clone();
    col()
        .key("prl-files")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((list.version, s.dark, s.mode, rows.len()))
        .virtual_rows(
            rows.len(),
            LINE,
            move |_: &State, i| Cow::Owned(row_key(keys[i])),
            move |s: &State, i| build_row(s, rows[i], &numbered),
        )
}
