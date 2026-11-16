use std::borrow::Cow;
use std::path::Path;

use gitpoulpe_core::{
    ChangeEntry, DiffLine, DiffLineKind, DocumentKind, EntryKind, StatusCode, display_label,
};
use rune::{
    Align, Cx, El, FileDialogOptions, GrowthSpec, ScrollSpec, Tx, col, el, row, text, text_input,
};

use crate::state::{Screen, State, ViewKind};
use crate::theme::{self, Palette};

const LIST_WIDTH: f32 = 372.0;
const LIST_ROW_HEIGHT: f32 = 56.0;
const LINE_HEIGHT: f32 = 25.0;

pub fn view(s: &State, _: &mut Cx<State>) -> El {
    let p = s.palette;
    let body = match &s.screen {
        Screen::Ready => workspace(s),
        Screen::Welcome => message_screen(
            &p,
            "Ouvrir un dépôt Git",
            "Choisissez un dossier, ou lancez gitpoulpe avec le chemin du dépôt.",
            true,
        ),
        Screen::Opening(path) => message_screen(&p, "Ouverture…", &display_label(path), false),
        Screen::Failed { path, message } => {
            let detail = if path.as_os_str().is_empty() {
                message.clone()
            } else {
                format!("{}\n{message}", display_label(path))
            };
            message_screen(&p, "Impossible d'ouvrir ce dépôt", &detail, true)
        }
    };
    col()
        .w_full()
        .h_full()
        .fill(p.bg)
        .child(top_bar(s))
        .child(body)
}

fn open_button(p: &Palette, primary: bool) -> El {
    let (fill, ink) = if primary {
        (p.accent, p.on_accent)
    } else {
        (p.raised, p.text)
    };
    el().key("open")
        .role("button")
        .label("Ouvrir un dépôt")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(12.0)
        .center()
        .radius(3.0)
        .fill(fill)
        .border(1.0, if primary { fill } else { p.line })
        .hover(p.hover)
        .on_press_tx(|_: &mut State, tx: &mut Tx| {
            let options = FileDialogOptions {
                title: "Ouvrir un dépôt Git".into(),
                ..Default::default()
            };
            tx.open_folder(options, |s: &mut State, result, _| {
                if let Ok(Some(path)) = result {
                    s.open_path(path);
                }
            });
        })
        .child(text("Ouvrir un dépôt…").font(theme::BODY_STRONG).color(ink))
}

fn top_bar(s: &State) -> El {
    let p = s.palette;
    let mut context = col().gap(2.0).grow(1.0).min_w(0.0);
    if let Some(repo) = &s.repo {
        let branch = repo.snapshot.branch.head.clone().unwrap_or_default();
        context = context
            .child(
                text(repo.name.clone())
                    .font(theme::BODY_STRONG)
                    .color(p.text)
                    .truncate(),
            )
            .child(text(branch).font(theme::SMALL).color(p.accent).truncate());
    }
    let theme_toggle = el()
        .key("theme")
        .role("button")
        .label("Changer de thème")
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(32.0)
        .px(10.0)
        .center()
        .radius(3.0)
        .hover(p.hover)
        .on_press(|s: &mut State| s.set_dark(!s.dark))
        .child(
            text(if s.dark {
                "Thème clair"
            } else {
                "Thème sombre"
            })
            .font(theme::BODY)
            .color(p.muted),
        );
    row()
        .w_full()
        .h(60.0)
        .px(20.0)
        .gap(20.0)
        .center_y()
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            row()
                .w(172.0)
                .child(text("git").font(theme::WORDMARK).color(p.text))
                .child(text("poulpe").font(theme::WORDMARK_BOLD).color(p.text)),
        )
        .child(context)
        .child(open_button(&p, false))
        .child(theme_toggle)
}

fn message_screen(p: &Palette, title: &str, detail: &str, with_open: bool) -> El {
    let mut card = col()
        .gap(10.0)
        .max_w(520.0)
        .child(text(title.to_owned()).font(theme::TITLE).color(p.text));
    card = card.child(
        text(detail.to_owned())
            .font(theme::BODY)
            .color(p.muted)
            .wrap(),
    );
    if with_open {
        card = card.child(open_button(p, true));
    }
    col().w_full().grow(1.0).center().child(card)
}

fn workspace(s: &State) -> El {
    row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(left_pane(s))
        .child(right_pane(s))
}

fn tab(s: &State, kind: ViewKind, label: &'static str) -> El {
    let p = s.palette;
    let active = s.view == kind;
    el().key(label)
        .role("tab")
        .label(format!("{label}, {} fichiers", s.count(kind)))
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .h(36.0)
        .px(2.0)
        .center_y()
        .hover(p.hover)
        .on_press(move |s: &mut State| s.set_view(kind))
        .child(
            row()
                .gap(6.0)
                .child(
                    text(label)
                        .font(if active {
                            theme::BODY_STRONG
                        } else {
                            theme::BODY
                        })
                        .color(if active { p.text } else { p.muted }),
                )
                .child(
                    text(s.count(kind).to_string())
                        .font(theme::SMALL)
                        .color(p.muted),
                ),
        )
}

fn left_pane(s: &State) -> El {
    let p = s.palette;
    let total = s.count(s.view);
    let filter = text_input(&s.filter, theme::BODY)
        .key("filter")
        .label("Rechercher un fichier")
        .single_line()
        .w_full()
        .h(34.0)
        .px(10.0)
        .py(8.0)
        .radius(3.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .color(p.text)
        .placeholder(format!("Nom ou chemin parmi {total} fichiers"), p.muted)
        .on_edit(|s: &mut State| &mut s.filter)
        .on_input_change(|s: &mut State, _| s.refilter());
    let counter = if s.files.len() == total {
        format!("{total} fichiers")
    } else {
        format!("{} sur {total} fichiers", s.files.len())
    };
    let list = col()
        .key("list-rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((s.layout_version, s.view, s.current))
        .virtual_rows(
            s.files.len(),
            LIST_ROW_HEIGHT,
            |s: &State, i| Cow::Owned(format!("f{}", s.files[i])),
            file_row,
        );
    col()
        .w(LIST_WIDTH)
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(
            col()
                .w_full()
                .px(16.0)
                .py(12.0)
                .gap(10.0)
                .child(
                    row()
                        .gap(18.0)
                        .child(tab(s, ViewKind::Unstaged, "Non indexés"))
                        .child(tab(s, ViewKind::Staged, "Indexés")),
                )
                .child(filter)
                .child(text(counter).font(theme::SMALL).color(p.muted)),
        )
        .child(
            col()
                .key("list-viewport")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

fn status_of(entry: &ChangeEntry, view: ViewKind) -> (&'static str, &'static str) {
    if entry.is_conflict() {
        return ("Conflit", "!");
    }
    if entry.kind == EntryKind::Untracked {
        return ("Non suivi", "?");
    }
    let code = if view == ViewKind::Staged {
        entry.index
    } else {
        entry.worktree
    };
    match code {
        StatusCode::Added => ("Ajouté", "A"),
        StatusCode::Deleted => ("Supprimé", "D"),
        StatusCode::Renamed => ("Renommé", "R"),
        StatusCode::Copied => ("Copié", "C"),
        StatusCode::TypeChanged => ("Type modifié", "T"),
        _ => ("Modifié", "M"),
    }
}

fn split_path(path: &Path) -> (String, String) {
    let name = path
        .file_name()
        .map(|n| display_label(Path::new(n)))
        .unwrap_or_default();
    let dir = path.parent().map(display_label).unwrap_or_default();
    (name, dir)
}

fn file_row(s: &State, position: usize) -> El {
    let p = s.palette;
    let Some((index, entry)) = s.entry_at(position) else {
        return el().h(LIST_ROW_HEIGHT);
    };
    let (name, dir) = split_path(&entry.path);
    let (word, letter) = status_of(entry, s.view);
    let selected = s.current == Some(position);
    let fill = if selected { p.selected } else { p.panel };
    el().key(format!("fr{index}"))
        .role("button")
        .label(format!("{}, {word}", display_label(&entry.path)))
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(LIST_ROW_HEIGHT)
        .fill(fill)
        .hover(p.hover)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
            s.current = Some(position);
            tx.scroll_into_view(format!("h{index}"));
        })
        .child(
            row()
                .w_full()
                .h_full()
                .child(
                    el().w(if selected { 3.0 } else { 0.0 })
                        .h_full()
                        .fill(p.accent),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .px(16.0)
                        .py(8.0)
                        .gap(2.0)
                        .child(text(name).font(theme::BODY_MEDIUM).color(p.text).truncate())
                        .child(text(dir).font(theme::PATH_SMALL).color(p.muted).truncate()),
                )
                .child(
                    el().w(34.0)
                        .h_full()
                        .center()
                        .child(text(letter).font(theme::SMALL).color(p.muted)),
                ),
        )
}

fn right_pane(s: &State) -> El {
    let p = s.palette;
    if s.files.is_empty() {
        let (title, detail) = if s.count(s.view) == 0 {
            (
                "Aucune modification",
                "Cette vue ne contient aucun fichier modifié.".to_owned(),
            )
        } else {
            (
                "Aucun fichier ne correspond",
                format!("Aucun résultat pour « {} ».", s.filter.text.trim()),
            )
        };
        return col()
            .grow(1.0)
            .h_full()
            .center()
            .child(message_card(&p, title, &detail));
    }
    let rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows_version((s.layout_version, s.view, s.dark))
        .virtual_rows(
            s.layout.total as usize,
            LINE_HEIGHT,
            |s: &State, i| Cow::Owned(row_key(s, i)),
            build_row,
        )
        .on_visible_rows(|s: &mut State, range| s.on_visible(range));
    col().grow(1.0).h_full().min_w(0.0).min_h(0.0).child(
        col()
            .key("viewport")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(rows),
    )
}

fn message_card(p: &Palette, title: &str, detail: &str) -> El {
    col()
        .gap(8.0)
        .max_w(460.0)
        .child(
            text(title.to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(
            text(detail.to_owned())
                .font(theme::BODY)
                .color(p.muted)
                .wrap(),
        )
}

fn row_key(s: &State, i: usize) -> String {
    match s.locate(i as u64) {
        Some((position, 0)) => format!("h{}", s.files[position]),
        Some((position, local)) => format!("l{}:{}", s.files[position], local - 1),
        None => format!("x{i}"),
    }
}

fn build_row(s: &State, i: usize) -> El {
    let Some((position, local)) = s.locate(i as u64) else {
        return el().h(LINE_HEIGHT);
    };
    let Some((index, entry)) = s.entry_at(position) else {
        return el().h(LINE_HEIGHT);
    };
    if local == 0 {
        return header_row(s, index, entry);
    }
    body_row(s, index, local - 1)
}

fn header_row(s: &State, index: usize, entry: &ChangeEntry) -> El {
    let p = s.palette;
    let (word, _) = status_of(entry, s.view);
    let mut meta = word.to_owned();
    if let Some(m) = s.doc(index).and_then(|d| d.meta.as_ref())
        && m.added + m.removed > 0
    {
        meta = format!("{word} · +{} −{}", m.added, m.removed);
    }
    let mut path = display_label(&entry.path);
    if let Some(original) = &entry.original_path {
        path = format!("{} → {path}", display_label(original));
    }
    col()
        .key(format!("h{index}"))
        .w_full()
        .py(6.0)
        .px(14.0)
        .child(
            col()
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .child(text(path).font(theme::PATH).color(p.text).truncate())
                .child(text(meta).font(theme::TINY).color(p.muted)),
        )
}

fn notice(s: &State, index: usize, key: String, message: String, tone: Tone) -> El {
    let p = s.palette;
    let ink = if tone == Tone::Error { p.red } else { p.muted };
    row()
        .key(key)
        .w_full()
        .px(28.0)
        .py(10.0)
        .child(text(message).font(theme::BODY).color(ink).wrap())
        .key(format!("n{index}"))
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tone {
    Plain,
    Error,
}

fn body_row(s: &State, index: usize, line: u64) -> El {
    let key = format!("l{index}:{line}");
    let Some(doc) = s.doc(index) else {
        return placeholder(s, key, "Chargement…");
    };
    if let Some(error) = &doc.error {
        return notice(
            s,
            index,
            key,
            format!("Lecture impossible : {error}"),
            Tone::Error,
        );
    }
    let Some(meta) = &doc.meta else {
        return placeholder(s, key, "Chargement…");
    };
    match &meta.kind {
        Some(DocumentKind::Text) | None => match s.line(index, line) {
            Some(diff_line) => line_row(&s.palette, key, diff_line),
            None => placeholder(s, key, "…"),
        },
        Some(other) => notice(
            s,
            index,
            key,
            kind_message(other),
            if matches!(other, DocumentKind::Unreadable(_)) {
                Tone::Error
            } else {
                Tone::Plain
            },
        ),
    }
}

fn kind_message(kind: &DocumentKind) -> String {
    match kind {
        DocumentKind::Binary => "Fichier binaire : son contenu n'est pas affiché.".to_owned(),
        DocumentKind::Conflict => {
            "Conflit de fusion non résolu : le contenu n'est pas affiché.".to_owned()
        }
        DocumentKind::ModeChange { old, new } => {
            format!("Mode modifié : {old} → {new}, sans changement de contenu.")
        }
        DocumentKind::RenameOnly { from, to } => {
            format!(
                "Renommé de {} vers {}, sans changement de contenu.",
                display_label(from),
                display_label(to)
            )
        }
        DocumentKind::NoChanges => "Ce fichier n'a plus de changement dans cette vue.".to_owned(),
        DocumentKind::Unreadable(diagnostic) => format!("Lecture impossible : {diagnostic}"),
        DocumentKind::Text => String::new(),
    }
}

fn placeholder(s: &State, key: String, label: &str) -> El {
    row()
        .key(key)
        .w_full()
        .h(LINE_HEIGHT)
        .px(28.0)
        .center_y()
        .child(
            text(label.to_owned())
                .font(theme::BODY)
                .color(s.palette.muted),
        )
}

fn line_row(p: &Palette, key: String, line: &DiffLine) -> El {
    let number = |value: Option<u32>| {
        text(value.map(|n| n.to_string()).unwrap_or_default())
            .font(theme::CODE)
            .color(p.muted)
            .w(46.0)
            .align(Align::End)
    };
    let content = String::from_utf8_lossy(&line.text).into_owned();
    match line.kind {
        DiffLineKind::HunkHeader => row()
            .key(key)
            .w_full()
            .h(LINE_HEIGHT)
            .px(14.0)
            .center_y()
            .fill(p.panel)
            .child(el().w(100.0))
            .child(text(content).font(theme::CODE).color(p.muted).truncate()),
        DiffLineKind::NoNewlineAtEof => row()
            .key(key)
            .w_full()
            .h(LINE_HEIGHT)
            .px(14.0)
            .center_y()
            .child(el().w(116.0))
            .child(
                text("\\ Pas de saut de ligne à la fin du fichier")
                    .font(theme::CODE)
                    .color(p.muted)
                    .truncate(),
            ),
        kind => {
            let (fill, sign, ink) = match kind {
                DiffLineKind::Added => (Some(p.added), "+", p.green),
                DiffLineKind::Removed => (Some(p.removed), "−", p.red),
                _ => (None, " ", p.muted),
            };
            let mut line_el = row()
                .key(key)
                .w_full()
                .h(LINE_HEIGHT)
                .px(14.0)
                .center_y()
                .child(number(line.old_number))
                .child(number(line.new_number))
                .child(
                    text(sign)
                        .font(theme::CODE)
                        .color(ink)
                        .w(22.0)
                        .align(Align::Center),
                )
                .child(
                    text(content)
                        .font(theme::CODE)
                        .color(p.text)
                        .selectable(true)
                        .truncate(),
                );
            if let Some(fill) = fill {
                line_el = line_el.fill(fill);
            }
            line_el
        }
    }
}
