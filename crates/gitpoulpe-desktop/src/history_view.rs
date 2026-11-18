use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::history_state::{HistoryStatus, format_date};
use crate::state::State;
use crate::theme;

const ROW_HEIGHT: f32 = 28.0;
const GLYPH_CELL: f32 = 8.0;

fn commit_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let history = &s.history;
    let (Some(commit), Some(graph)) = (history.commits.get(index), history.rows.get(index)) else {
        return el().h(ROW_HEIGHT);
    };
    let selected = history.selected == Some(index);
    let glyphs = gitpoulpe_core::glyphs(graph, history.width);
    let short: String = commit.oid.chars().take(8).collect();
    let mut line = row()
        .w_full()
        .h(ROW_HEIGHT)
        .px(12.0)
        .gap(10.0)
        .center_y()
        .child(
            el().w(history.width.max(1) as f32 * 2.0 * GLYPH_CELL)
                .child(text(glyphs).font(theme::CODE).color(p.accent)),
        )
        .child(
            text(commit.subject.clone())
                .font(theme::BODY)
                .color(p.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if !commit.refs.is_empty() {
        line = line.child(
            text(commit.refs.join(" · "))
                .font(theme::SMALL)
                .color(p.accent)
                .max_w(220.0)
                .truncate(),
        );
    }
    line = line
        .child(
            text(commit.author.clone())
                .font(theme::SMALL)
                .color(p.muted)
                .w(130.0)
                .truncate(),
        )
        .child(
            text(format_date(commit.timestamp))
                .font(theme::SMALL)
                .color(p.muted)
                .w(116.0),
        )
        .child(text(short).font(theme::PATH_SMALL).color(p.muted).w(64.0));
    el().key(format!("hr{index}"))
        .role("option")
        .label(format!("{}, {}", commit.subject, commit.author))
        .focusable(true)
        .focus_ring(2.0, -2.0, p.accent)
        .w_full()
        .h(ROW_HEIGHT)
        .fill(if selected { p.selected } else { p.bg })
        .hover(p.hover)
        .on_press(move |s: &mut State| s.select_commit(index))
        .child(line)
}

fn detail(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .w(340.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(8.0)
        .fill(p.panel)
        .border(1.0, p.line);
    let Some(commit) = s.history.selected.and_then(|i| s.history.commits.get(i)) else {
        return panel.child(
            text("Sélectionne un commit pour voir ses détails.".to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    };
    let parents = if commit.parents.is_empty() {
        "aucun (commit racine)".to_owned()
    } else {
        commit
            .parents
            .iter()
            .map(|oid| oid.chars().take(8).collect::<String>())
            .collect::<Vec<_>>()
            .join(", ")
    };
    panel = panel
        .child(
            text(commit.subject.clone())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .wrap(),
        )
        .child(
            text(format!(
                "{} · {} UTC",
                commit.author,
                format_date(commit.timestamp)
            ))
            .font(theme::SMALL)
            .color(p.muted)
            .wrap(),
        )
        .child(
            text(format!("Commit {}", commit.oid))
                .key("hist-detail-oid")
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap()
                .selectable(true),
        )
        .child(
            text(format!("Parents : {parents}"))
                .font(theme::PATH_SMALL)
                .color(p.muted)
                .wrap(),
        );
    if !commit.refs.is_empty() {
        panel = panel.child(
            text(format!("Références : {}", commit.refs.join(", ")))
                .font(theme::SMALL)
                .color(p.accent)
                .wrap(),
        );
    }
    panel
}

fn message(s: &State, title: &str, detail: &str) -> El {
    let p = s.palette;
    col().w_full().grow(1.0).center().child(
        col()
            .gap(8.0)
            .max_w(520.0)
            .child(text(title.to_owned()).font(theme::TITLE).color(p.text))
            .child(
                text(detail.to_owned())
                    .font(theme::BODY)
                    .color(p.muted)
                    .wrap(),
            ),
    )
}

pub fn history_screen(s: &State) -> El {
    let p = s.palette;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .child(
            text(format!(
                "Historique · {} commits chargés",
                s.history.commits.len()
            ))
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0),
        )
        .child(
            el().key("history-close")
                .role("button")
                .label("Fermer l'historique")
                .focusable(true)
                .focus_ring(2.0, 2.0, p.accent)
                .h(32.0)
                .px(12.0)
                .center()
                .radius(3.0)
                .fill(p.raised)
                .border(1.0, p.line)
                .hover(p.hover)
                .on_press(|s: &mut State| s.close_history())
                .child(text("Fermer".to_owned()).font(theme::BODY).color(p.text)),
        );
    let body = match &s.history.status {
        HistoryStatus::Idle | HistoryStatus::Loading if s.history.commits.is_empty() => message(
            s,
            "Chargement de l'historique…",
            "Lecture des premiers commits.",
        ),
        HistoryStatus::NoCommits => message(
            s,
            "Aucun commit",
            "Ce dépôt n'a pas encore de commit : crée le premier depuis l'écran des changements.",
        ),
        HistoryStatus::Failed(error) => message(s, "Lecture de l'historique impossible", error),
        _ => {
            let list = col()
                .key("hist-rows")
                .role("listbox")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows_version((s.history.version, s.history.selected, s.dark))
                .virtual_rows(
                    s.history.commits.len(),
                    ROW_HEIGHT,
                    |_: &State, i| Cow::Owned(format!("hk{i}")),
                    commit_row,
                )
                .on_visible_rows(|s: &mut State, range| s.history_visible(range));
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(
                    col()
                        .key("hist-viewport")
                        .grow(1.0)
                        .min_w(0.0)
                        .h_full()
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(list),
                )
                .child(detail(s))
        }
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
