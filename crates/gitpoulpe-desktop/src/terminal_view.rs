use gitpoulpe_terminal::{Run, Snapshot, TermColor};
use rune::color::hex;
use rune::{Color, El, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::terminal::{CELL_HEIGHT, CELL_WIDTH};
use crate::theme;
use crate::widgets::action;

const ANSI: [&str; 16] = [
    "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
    "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];

fn rgb(r: u8, g: u8, b: u8) -> Color {
    hex(&format!("#{r:02x}{g:02x}{b:02x}")).expect("a formatted color is valid hex")
}

fn color_of(color: TermColor, default: Color) -> Color {
    match color {
        TermColor::Default => default,
        TermColor::Rgb(r, g, b) => rgb(r, g, b),
        TermColor::Indexed(i) if i < 16 => {
            hex(ANSI[i as usize]).expect("palette entries are valid")
        }
        TermColor::Indexed(i) if i < 232 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        TermColor::Indexed(i) => {
            let v = 8 + 10 * (i - 232);
            rgb(v, v, v)
        }
    }
}

fn with_cursor(runs: &[Run], column: u16) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut at = 0u16;
    for run in runs {
        let mut plain = String::new();
        for ch in run.text.chars() {
            if at == column {
                if !plain.is_empty() {
                    out.push(Run {
                        text: std::mem::take(&mut plain),
                        ..run.clone()
                    });
                }
                out.push(Run {
                    text: ch.to_string(),
                    inverse: !run.inverse,
                    ..run.clone()
                });
            } else {
                plain.push(ch);
            }
            at += 1;
        }
        if !plain.is_empty() {
            out.push(Run {
                text: plain,
                ..run.clone()
            });
        }
    }
    out
}

fn run_el(s: &State, run: &Run) -> El {
    let p = s.palette;
    let (mut fg, mut bg) = (color_of(run.fg, p.text), color_of(run.bg, p.panel));
    if run.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    let label = text(run.text.clone()).font(theme::TERMINAL).color(fg);
    if run.bg == TermColor::Default && !run.inverse {
        label
    } else {
        row().fill(bg).child(label)
    }
}

fn line(s: &State, index: usize, runs: &[Run], cursor: Option<u16>) -> El {
    let runs = match cursor {
        Some(column) => with_cursor(runs, column),
        None => runs.to_vec(),
    };
    let mut line = row().key(format!("tm-row{index}")).h(CELL_HEIGHT).gap(0.0);
    for run in &runs {
        line = line.child(run_el(s, run));
    }
    line
}

fn screen(s: &State, snapshot: &Snapshot) -> El {
    let p = s.palette;
    let mut rows = col().w_full().gap(0.0);
    for (index, runs) in snapshot.rows.iter().enumerate() {
        let cursor = snapshot
            .cursor
            .filter(|(row, _)| *row as usize == index)
            .map(|(_, column)| column);
        rows = rows.child(line(s, index, runs, cursor));
    }
    let session = s.terminal.session.clone();
    col()
        .key("tm-screen")
        .role("region")
        .label("Terminal")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(8.0)
        .py(4.0)
        .fill(p.panel)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(move |at| {
            if let Some(session) = &session {
                let cols = ((at.viewport[0] - 16.0) / CELL_WIDTH).floor().max(2.0) as u16;
                let rows = ((at.viewport[1] - 8.0) / CELL_HEIGHT).floor().max(2.0) as u16;
                session.resize(rows, cols);
            }
        })
        .child(rows)
}

pub fn terminal_screen(s: &State) -> El {
    let p = s.palette;
    let terminal = &s.terminal;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text(format!(
                "Terminal · {} · {}",
                terminal.shell, terminal.directory
            ))
            .key("tm-title")
            .font(theme::BODY_STRONG)
            .color(p.text)
            .grow(1.0)
            .truncate(),
        )
        .child(
            action(s, "tm-copy", "Copier l'écran", false)
                .on_press(|s: &mut State| s.terminal_copy()),
        )
        .child(
            action(s, "tm-interrupt", "Interrompre (Ctrl+C)", false)
                .on_press(|s: &mut State| s.terminal_interrupt()),
        )
        .child(action(s, "tm-close", "Fermer", false).on_press(|s: &mut State| s.close_terminal()));
    let mut body = col().w_full().grow(1.0).min_h(0.0).child(header);
    if let Some(error) = &terminal.error {
        return body.child(
            col().w_full().px(20.0).child(
                text(format!("Terminal indisponible : {error}"))
                    .key("tm-error")
                    .font(theme::BODY)
                    .color(p.red)
                    .wrap(),
            ),
        );
    }
    let Some(session) = &terminal.session else {
        return body;
    };
    let snapshot = session.snapshot();
    let mut notes = col().w_full().px(20.0).gap(4.0);
    if let Some(code) = snapshot.exited {
        notes = notes.child(
            text(format!(
                "Session terminée (code {code}) : fermez le terminal pour revenir au dépôt."
            ))
            .key("tm-exited")
            .font(theme::SMALL)
            .color(p.red)
            .wrap(),
        );
    }
    if let Some(notice) = &terminal.notice {
        notes = notes.child(
            text(notice.clone())
                .key("tm-notice")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    body = body.child(notes);
    body.child(screen(s, &snapshot)).child(el().h(0.0))
}
