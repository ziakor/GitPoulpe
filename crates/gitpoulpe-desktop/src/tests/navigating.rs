use rune::Runtime;

use super::{click_at, rect_of, sized_runtime, step};
use crate::nav::Place;
use crate::state::State;

fn go(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("nav-") && !matches!(key, "nav-show" | "nav-hide") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("nav-list", target));
        step(rt);
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not in the menu"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
    for _ in 0..3 {
        step(rt);
    }
}

#[test]
fn every_menu_entry_opens_its_screen_in_one_click_and_marks_it_as_current() {
    let (_repo, mut rt) = sized_runtime(6, 1440.0, 900.0);
    assert_eq!(rt.state.place(), Place::Changes);
    for (key, place) in [
        ("history", Place::History),
        ("ignored", Place::Ignored),
        ("nav-issues", Place::Issues),
        ("nav-notices", Place::Notices),
        ("nav-branches", Place::Branches),
        ("nav-tags", Place::Tags),
        ("nav-remotes", Place::Remotes),
        ("nav-stashes", Place::Stashes),
        ("nav-worktrees", Place::Worktrees),
        ("nav-accounts", Place::Accounts),
        ("profiles", Place::Profiles),
        ("nav-tools", Place::Tools),
        ("nav-shortcuts", Place::Shortcuts),
        ("nav-changes", Place::Changes),
    ] {
        go(&mut rt, key);
        assert_eq!(rt.state.place(), place, "{key}");
    }

    assert!(
        rect_of(&rt, "nav-reflog").is_none(),
        "advanced tools start folded"
    );
    go(&mut rt, "nav-more");
    for (key, place) in [
        ("nav-reflog", Place::Reflog),
        ("nav-undo", Place::Undo),
        ("nav-hooks", Place::Hooks),
        ("nav-lfs", Place::Lfs),
        ("nav-submodules", Place::Submodules),
        ("nav-gitflow", Place::Gitflow),
    ] {
        go(&mut rt, key);
        assert_eq!(rt.state.place(), place, "{key}");
    }

    assert!(
        rect_of(&rt, "nav-favorites").is_none(),
        "repositories start folded"
    );
    go(&mut rt, "nav-repositories");
    for (key, place) in [
        ("nav-favorites", Place::Favorites),
        ("nav-workspaces", Place::Workspaces),
        ("nav-overview", Place::Overview),
    ] {
        go(&mut rt, key);
        assert_eq!(rt.state.place(), place, "{key}");
        assert!(
            rect_of(&rt, key).is_some(),
            "{key} stays in view while current"
        );
    }
}

#[test]
fn the_menu_folds_away_in_a_small_window_and_comes_back_on_demand() {
    let (_repo, mut rt) = sized_runtime(6, 900.0, 600.0);
    assert!(rect_of(&rt, "nav").is_none());
    go(&mut rt, "nav-show");
    assert!(rect_of(&rt, "nav").is_some());
    go(&mut rt, "nav-hide");
    assert!(rect_of(&rt, "nav").is_none());
}

fn footer(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter_map(|e| e.text)
        .find(|t| t.ends_with(" : palette de commandes"))
        .unwrap_or_default()
}

fn text_of(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .and_then(|e| e.text)
}

#[test]
fn the_footer_only_lists_the_keys_that_act_where_you_are() {
    let (_repo, mut rt) = sized_runtime(6, 1440.0, 900.0);
    let palette = format!(
        "{} : palette de commandes",
        rt.state.shortcut_for("palette")
    );
    let stage = format!("{} : indexer", rt.state.shortcut_for("stage"));
    assert!(footer(&rt).contains(&stage), "{}", footer(&rt));
    go(&mut rt, "nav-branches");
    assert_eq!(footer(&rt), format!("Échap : fermer l'écran · {palette}"));
    go(&mut rt, "nav-changes");
    assert!(footer(&rt).contains(&stage));
}

#[test]
fn the_file_count_appears_only_while_a_filter_hides_files() {
    let (_repo, mut rt) = sized_runtime(6, 1440.0, 900.0);
    assert_eq!(text_of(&rt, "filter-count"), None);
    let total = rt.state.count(rt.state.view);
    rt.update(|s, _| {
        s.filter.text = "file_0001".to_owned();
        s.filter_changed();
    });
    for _ in 0..3 {
        step(&mut rt);
    }
    assert_eq!(rt.state.files.len(), 1);
    assert_eq!(
        text_of(&rt, "filter-count"),
        Some(format!("1 sur {total} fichiers"))
    );
}
