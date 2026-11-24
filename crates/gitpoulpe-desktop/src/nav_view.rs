use rune::{El, ScrollSpec, Tx, col, el, row, text};

use crate::nav::Place;
use crate::palette::COMMANDS;
use crate::state::{State, ViewKind};
use crate::theme;

pub const NAV_WIDTH: f32 = 208.0;

enum Go {
    Command(&'static str),
    Changes,
    PullList,
}

struct Item {
    key: &'static str,
    label: &'static str,
    place: Place,
    go: Go,
}

const fn item(key: &'static str, label: &'static str, place: Place, go: Go) -> Item {
    Item {
        key,
        label,
        place,
        go,
    }
}

const WORK: &[Item] = &[
    item("nav-changes", "Modifications", Place::Changes, Go::Changes),
    item(
        "history",
        "Historique",
        Place::History,
        Go::Command("history"),
    ),
    item(
        "ignored",
        "Fichiers ignorés",
        Place::Ignored,
        Go::Command("ignored"),
    ),
];

const TEAM: &[Item] = &[
    item("nav-pulls", "Pull requests", Place::Pulls, Go::PullList),
    item("nav-issues", "Issues", Place::Issues, Go::Command("issues")),
    item(
        "nav-notices",
        "Notifications",
        Place::Notices,
        Go::Command("notices"),
    ),
];

const REFS: &[Item] = &[
    item(
        "nav-branches",
        "Branches",
        Place::Branches,
        Go::Command("branches"),
    ),
    item("nav-tags", "Tags", Place::Tags, Go::Command("tags")),
    item(
        "nav-remotes",
        "Dépôts distants",
        Place::Remotes,
        Go::Command("remotes"),
    ),
    item(
        "nav-stashes",
        "Remisages",
        Place::Stashes,
        Go::Command("stashes"),
    ),
    item(
        "nav-worktrees",
        "Worktrees et terminal",
        Place::Worktrees,
        Go::Command("worktrees"),
    ),
];

const OPERATION: &[Item] = &[item(
    "nav-conflicts",
    "Résoudre les conflits",
    Place::Conflicts,
    Go::Command("conflicts"),
)];

const ADVANCED: &[Item] = &[
    item("nav-reflog", "Reflog", Place::Reflog, Go::Command("reflog")),
    item(
        "nav-undo",
        "Annuler ou rétablir",
        Place::Undo,
        Go::Command("undo"),
    ),
    item("nav-hooks", "Hooks", Place::Hooks, Go::Command("hooks")),
    item("nav-lfs", "Git LFS", Place::Lfs, Go::Command("lfs")),
    item(
        "nav-submodules",
        "Sous-modules",
        Place::Submodules,
        Go::Command("submodules"),
    ),
    item(
        "nav-gitflow",
        "Gitflow",
        Place::Gitflow,
        Go::Command("gitflow"),
    ),
    item(
        "nav-patch-import",
        "Importer un patch",
        Place::Patches,
        Go::Command("patch-import"),
    ),
    item(
        "nav-patch-export",
        "Exporter en patch",
        Place::Patches,
        Go::Command("patch-export"),
    ),
];

const REPOSITORIES: &[Item] = &[
    item(
        "nav-open",
        "Ouvrir un dépôt…",
        Place::Other,
        Go::Command("open-repository"),
    ),
    item(
        "nav-clone",
        "Cloner un dépôt",
        Place::Other,
        Go::Command("clone"),
    ),
    item(
        "nav-favorites",
        "Favoris",
        Place::Favorites,
        Go::Command("favorites"),
    ),
    item(
        "nav-workspaces",
        "Espaces de travail",
        Place::Workspaces,
        Go::Command("workspaces"),
    ),
    item(
        "nav-overview",
        "Vue multi-dépôts",
        Place::Overview,
        Go::Command("overview"),
    ),
];

const SETTINGS: &[Item] = &[
    item(
        "nav-accounts",
        "Comptes fournisseurs",
        Place::Accounts,
        Go::Command("accounts"),
    ),
    item(
        "profiles",
        "Profils Git",
        Place::Profiles,
        Go::Command("profiles"),
    ),
    item(
        "nav-tools",
        "Outils externes",
        Place::Tools,
        Go::Command("tools"),
    ),
    item(
        "nav-shortcuts",
        "Raccourcis clavier",
        Place::Shortcuts,
        Go::Command("shortcuts"),
    ),
];

fn available(s: &State, go: &Go) -> bool {
    match go {
        Go::Command(id) => COMMANDS
            .iter()
            .find(|c| c.id == *id)
            .is_some_and(|c| (c.available)(s)),
        Go::Changes | Go::PullList => s.repo.is_some(),
    }
}

fn run(s: &mut State, tx: &mut Tx, item: &Item) {
    match item.go {
        Go::Command(id) => {
            if let Some(command) = COMMANDS.iter().find(|c| c.id == id) {
                if item.place != Place::Other {
                    s.close_screens();
                }
                (command.run)(s, tx);
            }
        }
        Go::Changes => s.go_changes(),
        Go::PullList => s.open_pull_list_direct(),
    }
}

fn count(s: &State, place: Place) -> Option<usize> {
    let n = match place {
        Place::Changes => s.count(ViewKind::Commit),
        Place::Notices => s.notices_unread(),
        _ => 0,
    };
    (n > 0).then_some(n)
}

fn entry(s: &State, item: &'static Item, here: Place) -> El {
    let p = s.palette;
    let active = item.place == here && item.place != Place::Other;
    let rule = if active {
        el().w(2.0).h_full().fill(p.accent)
    } else {
        el().w(2.0).h_full()
    };
    let mut line = row()
        .key(item.key)
        .role("link")
        .label(item.label)
        .focus_selected(active)
        .focusable(true)
        .focus_ring(2.0, 0.0, p.accent)
        .w_full()
        .h(30.0)
        .center_y()
        .hover(p.hover)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx, item))
        .child(rule)
        .child(
            text(item.label)
                .font(if active {
                    theme::BODY_MEDIUM
                } else {
                    theme::BODY
                })
                .color(p.text)
                .pl(12.0)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if let Some(n) = count(s, item.place) {
        line = line.child(
            text(n.to_string())
                .font(theme::SMALL)
                .color(p.muted)
                .pr(14.0),
        );
    }
    if active {
        line = line.fill(p.selected);
    }
    line
}

fn section(s: &State, title: &str, items: &'static [Item], here: Place) -> Option<El> {
    let p = s.palette;
    let shown: Vec<&'static Item> = items.iter().filter(|i| available(s, &i.go)).collect();
    if shown.is_empty() {
        return None;
    }
    let mut block = col().w_full().pb(10.0);
    if !title.is_empty() {
        block = block.child(
            text(title.to_owned())
                .font(theme::SMALL)
                .color(p.muted)
                .px(14.0)
                .pt(10.0)
                .pb(4.0),
        );
    }
    for item in shown {
        block = block.child(entry(s, item, here));
    }
    Some(block)
}

fn fold(
    s: &State,
    key: &'static str,
    title: &'static str,
    open: bool,
    toggle: fn(&mut State),
) -> El {
    let p = s.palette;
    row()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(2.0, 0.0, p.accent)
        .w_full()
        .h(30.0)
        .center_y()
        .px(14.0)
        .hover(p.hover)
        .on_press(move |s: &mut State| toggle(s))
        .child(
            text(format!("{title} {}", if open { "▾" } else { "▸" }))
                .font(theme::SMALL)
                .color(p.muted),
        )
}

pub fn sidebar(s: &State) -> El {
    let p = s.palette;
    let here = s.place();
    let mut column = col()
        .key("nav")
        .role("navigation")
        .label("Navigation")
        .w(NAV_WIDTH)
        .h_full()
        .fill(p.panel)
        .border_sides([0.0, 1.0, 0.0, 0.0], p.line);
    let mut list = col().key("nav-list").w_full().pt(4.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (title, items) in [
        ("Travail", WORK),
        ("Collaboration", TEAM),
        ("Références", REFS),
        ("Opération en cours", OPERATION),
    ] {
        if let Some(block) = section(s, title, items, here) {
            list = list.child(block);
        }
    }
    if s.repo.is_some() {
        let open = s.nav.more || here.advanced();
        list = list.child(fold(
            s,
            "nav-more",
            "Outils avancés",
            open,
            State::toggle_nav_more,
        ));
        if open && let Some(block) = section(s, "", ADVANCED, here) {
            list = list.child(block);
        }
    }
    let open = s.nav.repositories || here.repositories() || s.repo.is_none();
    list = list.child(fold(
        s,
        "nav-repositories",
        "Dépôts",
        open,
        State::toggle_nav_repositories,
    ));
    if open && let Some(block) = section(s, "", REPOSITORIES, here) {
        list = list.child(block);
    }
    column = column.child(list.grow(1.0).min_h(0.0));
    if let Some(block) = section(s, "Réglages", SETTINGS, here) {
        column = column.child(block.border_sides([1.0, 0.0, 0.0, 0.0], p.line));
    }
    column = column.child(
        row()
            .w_full()
            .px(8.0)
            .py(6.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], p.line)
            .child(
                el().key("nav-hide")
                    .role("button")
                    .label("Masquer le menu")
                    .focusable(true)
                    .focus_ring(2.0, 2.0, p.accent)
                    .px(6.0)
                    .py(4.0)
                    .radius(3.0)
                    .hover(p.hover)
                    .on_press(|s: &mut State| s.toggle_nav())
                    .child(
                        text("Masquer le menu".to_owned())
                            .font(theme::SMALL)
                            .color(p.muted),
                    ),
            ),
    );
    column
}
