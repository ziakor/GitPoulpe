use rune::{El, ScrollSpec, col, row, text};

use crate::overview::RowState;
use crate::state::State;
use crate::theme;
use crate::widgets::{action, counted, screen_head};

fn summary(overview: &gitpoulpe_git::RepoOverview) -> String {
    let branch = overview
        .branch
        .clone()
        .unwrap_or_else(|| "HEAD détachée".to_owned());
    let distance = match (overview.upstream.as_ref(), overview.ahead, overview.behind) {
        (Some(_), Some(a), Some(b)) => format!("↑{a} ↓{b}"),
        (Some(_), _, _) => "suivi distant".to_owned(),
        _ => "pas de suivi distant".to_owned(),
    };
    let mut parts = vec![
        format!("branche {branch}"),
        distance,
        counted(overview.staged as usize, "indexé", "indexés"),
        counted(overview.unstaged as usize, "modifié", "modifiés"),
        counted(overview.untracked as usize, "non suivi", "non suivis"),
    ];
    if overview.conflicts > 0 {
        parts.push(counted(overview.conflicts as usize, "conflit", "conflits"));
    }
    if let Some(operation) = overview.operation {
        parts.push(operation.to_owned());
    }
    parts.join(" · ")
}

pub fn overview_screen(s: &State) -> El {
    let p = s.palette;
    let screen = &s.overview;
    let candidates = s.overview_candidates();
    let header = screen_head(
        s,
        "ov-head",
        "Vue multi-dépôts",
        format!(
            "{} sur {}",
            counted(screen.selected.len(), "dépôt suivi", "dépôts suivis"),
            candidates.len()
        ),
        vec![
            action(s, "ov-refresh-all", "Actualiser tout", true)
                .on_press(|s: &mut State| s.overview_refresh_all()),
            action(s, "ov-all", "Tout sélectionner", false)
                .on_press(|s: &mut State| s.overview_select_all()),
            action(s, "ov-none", "Aucun", false).on_press(|s: &mut State| s.overview_select_none()),
        ],
    );
    let mut list = col().key("ov-list").w_full().gap(6.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    if let Some(notice) = &screen.notice {
        list = list.child(
            text(notice.clone())
                .key("ov-notice")
                .font(theme::SMALL)
                .color(p.red)
                .wrap(),
        );
    }
    if candidates.is_empty() {
        list = list.child(
            text("Aucun dépôt à afficher : ajoutez des dépôts à un espace de travail ou aux favoris.".to_owned())
                .key("ov-empty")
                .font(theme::SMALL)
                .color(p.muted)
                .wrap(),
        );
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let selected = screen.selected.contains(&candidate.path);
        let (toggle_path, open_path, refresh_path) = (
            candidate.path.clone(),
            candidate.path.clone(),
            candidate.path.clone(),
        );
        let (status, broken) = match screen.rows.get(&candidate.path).map(|r| &r.state) {
            _ if !selected => ("non sélectionné".to_owned(), false),
            None | Some(RowState::Loading) => ("lecture…".to_owned(), false),
            Some(RowState::Ready(overview)) => (summary(overview), false),
            Some(RowState::Failed(message)) => (message.clone(), true),
        };
        let saved = s
            .overview_saved_position(&candidate.path)
            .map(|(view, file)| {
                format!(
                    " · position enregistrée : vue {}{}",
                    if view == "staged" {
                        "indexés"
                    } else {
                        "non indexés"
                    },
                    file.map(|f| format!(", {f}")).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let mut line = row()
            .key(format!("ov-row{index}"))
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                crate::widgets::check(
                    s,
                    &format!("ov-toggle{index}"),
                    &format!("Suivre {}", candidate.name),
                    selected,
                )
                .on_press(move |s: &mut State| s.overview_toggle(&toggle_path)),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(format!(
                            "{} · {}",
                            candidate.name,
                            candidate.sources.join(", ")
                        ))
                        .font(theme::BODY_STRONG)
                        .color(p.text)
                        .truncate(),
                    )
                    .child(
                        text(candidate.path.clone())
                            .font(theme::PATH_SMALL)
                            .color(p.muted)
                            .truncate(),
                    )
                    .child(
                        text(format!("{status}{saved}"))
                            .font(theme::SMALL)
                            .color(if broken { p.red } else { p.text })
                            .truncate(),
                    ),
            );
        if selected {
            line = line.child(
                action(s, &format!("ov-refresh{index}"), "Actualiser", false)
                    .on_press(move |s: &mut State| s.overview_refresh_one(&refresh_path)),
            );
        }
        line = line.child(
            action(s, &format!("ov-open{index}"), "Ouvrir", false)
                .on_press(move |s: &mut State| s.overview_open(&open_path)),
        );
        list = list.child(line);
    }
    col().w_full().grow(1.0).min_h(0.0).child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(20.0)
            .gap(8.0)
            .child(list),
    )
}
