use std::borrow::Cow;

use gitpoulpe_core::{LfsFile, LfsFileState, LfsTransfer};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};

use crate::state::State;
use crate::theme;
use crate::widgets::action;

const ROW_HEIGHT: f32 = 44.0;

fn state_caption(state: LfsFileState) -> &'static str {
    match state {
        LfsFileState::Present => "contenu réel présent",
        LfsFileState::PointerOnly => "pointeur dans le dossier ; objet disponible localement",
        LfsFileState::Missing => "objet LFS absent : à récupérer",
        LfsFileState::Modified => "contenu modifié, différent de l'objet versionné",
        LfsFileState::NotConverted => "versionné en contenu réel malgré l'attribut LFS",
    }
}

fn file_row(s: &State, index: usize) -> El {
    let p = s.palette;
    let Some(file): Option<&LfsFile> = s.lfs.files.get(index) else {
        return el().h(ROW_HEIGHT);
    };
    let detail = match &file.pointer {
        Some(pointer) => format!(
            "{} · {} octets · {}",
            state_caption(file.state),
            pointer.size,
            pointer.oid.chars().take(10).collect::<String>()
        ),
        None => state_caption(file.state).to_owned(),
    };
    el().key(format!("lfs-row{index}"))
        .role("option")
        .label(file.path.display().to_string())
        .w_full()
        .h(ROW_HEIGHT)
        .px(16.0)
        .child(
            col()
                .w_full()
                .h(ROW_HEIGHT)
                .center_y()
                .gap(2.0)
                .child(
                    text(file.path.display().to_string())
                        .font(theme::PATH_SMALL)
                        .color(p.text)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .font(theme::SMALL)
                        .color(
                            if file.state == LfsFileState::Present
                                || file.state == LfsFileState::PointerOnly
                            {
                                p.muted
                            } else {
                                p.red
                            },
                        )
                        .truncate(),
                ),
        )
}

fn panel(s: &State) -> El {
    let p = s.palette;
    let mut panel = col()
        .key("lfs-panel")
        .w(440.0)
        .h_full()
        .px(16.0)
        .py(14.0)
        .gap(10.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(status) = &s.lfs.status else {
        return panel.child(
            text("Lecture…".to_owned())
                .font(theme::SMALL)
                .color(p.muted),
        );
    };
    panel = panel.child(
        text(if status.installed {
            format!(
                "Git LFS installé : {}",
                status.version.clone().unwrap_or_default()
            )
        } else {
            "Git LFS n'est pas installé".to_owned()
        })
        .key("lfs-installed")
        .font(theme::BODY_STRONG)
        .color(if status.installed { p.text } else { p.red })
        .wrap(),
    );
    if !status.installed {
        panel = panel.child(
            text(status.detail.clone())
                .key("lfs-howto")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
        if status.uses_lfs {
            panel = panel.child(
                text("Ce dépôt déclare des fichiers LFS : leurs pointeurs sont listés, mais aucun objet ne peut être transféré sans l'outil.".to_owned())
                    .font(theme::SMALL)
                    .color(p.red)
                    .wrap(),
            );
        }
    }
    if let Some((kind, phase, percent)) = &s.lfs.running {
        return panel
            .child(
                text(format!("{} · {phase} {percent} %", kind.caption()))
                    .key("lfs-running")
                    .font(theme::SMALL)
                    .color(p.muted),
            )
            .child(
                action(s, "lfs-cancel", "Annuler le transfert", false)
                    .on_press(|s: &mut State| s.lfs_cancel()),
            );
    }
    if let Some(done) = &s.lfs.done {
        panel = panel.child(
            text(done.clone())
                .key("lfs-done")
                .font(theme::SMALL)
                .color(p.text)
                .wrap(),
        );
    }
    if let Some(error) = &s.lfs.error {
        panel = panel.child(
            text(error.clone())
                .key("lfs-error")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if status.installed {
        panel = panel
            .child(
                action(s, "lfs-fetch", "Récupérer les objets (fetch)", false)
                    .on_press(|s: &mut State| s.lfs_run(LfsTransfer::Fetch)),
            )
            .child(
                action(s, "lfs-pull", "Télécharger dans le dossier (pull)", true)
                    .on_press(|s: &mut State| s.lfs_run(LfsTransfer::Pull)),
            )
            .child(
                action(s, "lfs-push", "Envoyer les objets (push)", false)
                    .on_press(|s: &mut State| s.lfs_run(LfsTransfer::Push)),
            );
    }
    panel
}

pub fn lfs_screen(s: &State) -> El {
    let p = s.palette;
    let title = if s.lfs.loading {
        "Git LFS · lecture…".to_owned()
    } else {
        format!("Git LFS · {} fichier(s)", s.lfs.files.len())
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
            action(s, "lfs-branches", "Branches", false)
                .on_press(|s: &mut State| s.open_branches()),
        )
        .child(action(s, "lfs-close", "Fermer", false).on_press(|s: &mut State| s.close_lfs()));
    let list: El = if s.lfs.files.is_empty() && !s.lfs.loading {
        col().w_full().grow(1.0).center().child(
            text("Aucun fichier géré par LFS dans ce dépôt.".to_owned())
                .key("lfs-empty")
                .font(theme::BODY)
                .color(p.muted),
        )
    } else {
        col()
            .key("lfs-viewport")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("lfs-rows")
                    .role("listbox")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows_version((s.lfs.version, s.dark))
                    .virtual_rows(
                        s.lfs.files.len(),
                        ROW_HEIGHT,
                        |_: &State, i| Cow::Owned(format!("lfsk{i}")),
                        file_row,
                    ),
            )
    };
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(list)
            .child(panel(s)),
    )
}
