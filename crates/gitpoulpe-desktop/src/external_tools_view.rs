use gitpoulpe_core::ToolKind;
use rune::{El, col, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::{action, field};

pub fn tool_status(s: &State) -> Option<El> {
    let p = s.palette;
    let tools = &s.tools;
    let message = tools
        .running
        .as_ref()
        .map(|label| {
            (
                format!("{label} en cours… fermez l'outil pour revenir ici."),
                false,
            )
        })
        .or_else(|| tools.status_error.clone().map(|m| (m, true)))
        .or_else(|| tools.status_notice.clone().map(|m| (m, false)))?;
    Some(
        text(message.0)
            .key("tool-status")
            .font(theme::SMALL)
            .color(if message.1 { p.red } else { p.muted })
            .wrap(),
    )
}

fn section(s: &State, kind: ToolKind) -> El {
    let p = s.palette;
    let (program_key, args_key) = match kind {
        ToolKind::Editor => ("tl-editor-program", "tl-editor-args"),
        ToolKind::Diff => ("tl-diff-program", "tl-diff-args"),
        ToolKind::Merge => ("tl-merge-program", "tl-merge-args"),
        ToolKind::Shell => ("tl-shell-program", "tl-shell-args"),
    };
    let variables = kind
        .variables()
        .iter()
        .map(|v| format!("{{{v}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let program = field(
        s,
        program_key,
        &format!("{} : exécutable", kind.caption()),
        &s.tools.form(kind).program,
        "ex. /usr/bin/outil",
    );
    let args = field(
        s,
        args_key,
        &format!("{} : arguments", kind.caption()),
        &s.tools.form(kind).args,
        kind.example(),
    );
    let (program, args) = match kind {
        ToolKind::Editor => (
            program.on_edit(|s: &mut State| &mut s.tools.editor.program),
            args.on_edit(|s: &mut State| &mut s.tools.editor.args),
        ),
        ToolKind::Diff => (
            program.on_edit(|s: &mut State| &mut s.tools.diff.program),
            args.on_edit(|s: &mut State| &mut s.tools.diff.args),
        ),
        ToolKind::Merge => (
            program.on_edit(|s: &mut State| &mut s.tools.merge.program),
            args.on_edit(|s: &mut State| &mut s.tools.merge.args),
        ),
        ToolKind::Shell => (
            program.on_edit(|s: &mut State| &mut s.tools.shell.program),
            args.on_edit(|s: &mut State| &mut s.tools.shell.args),
        ),
    };
    col()
        .w_full()
        .gap(6.0)
        .child(
            text(kind.caption().to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text),
        )
        .child(program)
        .child(args)
        .child(
            text(format!("Variables disponibles : {variables}"))
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        )
}

pub fn tools_screen(s: &State) -> El {
    let p = s.palette;
    let header = row()
        .w_full()
        .center_y()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .child(
            text("Outils externes".to_owned())
                .font(theme::BODY_STRONG)
                .color(p.text)
                .grow(1.0),
        )
        .child(
            action(s, "tl-save", "Enregistrer", true)
                .on_press(|s: &mut State| s.save_tools_config()),
        )
        .child(action(s, "tl-close", "Fermer", false).on_press(|s: &mut State| s.close_tools()));
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .py(8.0)
        .gap(14.0)
        .child(
            text("L'exécutable est lancé directement, sans shell : chaque argument est transmis tel quel, espaces et caractères spéciaux compris. Utilisez des guillemets pour regrouper des mots.".to_owned())
                .key("tl-help")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    if let Some(error) = &s.tools.error {
        body = body.child(
            text(error.clone())
                .key("tl-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if let Some(notice) = &s.tools.notice {
        body = body.child(
            text(notice.clone())
                .key("tl-notice")
                .font(theme::SMALL)
                .color(p.muted),
        );
    }
    for kind in ToolKind::ALL {
        body = body.child(section(s, kind));
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(header)
        .child(body)
}
