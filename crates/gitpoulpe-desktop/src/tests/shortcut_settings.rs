use std::collections::BTreeMap;

use rune::{Key, KeyEvent, Mods, Runtime};

use super::{click_at, rect_of, step};
use crate::state::{State, ViewKind};

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    step(rt);
}

fn text(rt: &mut Runtime<State>, value: &str) {
    press(rt, Key::Text(value.to_owned()), Mods::default());
}

fn ctrl(rt: &mut Runtime<State>, key: char) {
    press(
        rt,
        Key::Command(key),
        Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    );
}

fn customize(rt: &mut Runtime<State>, id: &'static str, chord: &str) {
    rt.state.shortcut_select(id);
    click(rt, "sc-modify");
    step(rt);
    assert!(rt.state.shortcuts.capturing.is_some());
    if let Some(key) = chord.strip_prefix("Ctrl+") {
        ctrl(rt, key.to_lowercase().chars().next().unwrap());
    } else {
        text(rt, chord);
    }
}

fn settings() -> (tempfile::TempDir, Runtime<State>) {
    let (dir, mut rt) = super::keyboard_runtime(3);
    rt.state.open_shortcuts();
    step(&mut rt);
    (dir, rt)
}

#[test]
fn the_effective_shortcuts_are_listed_and_searchable() {
    let (_dir, mut rt) = settings();
    assert!(shows(&rt, "Fichier suivant") && shows(&rt, "Palette de commandes"));
    assert!(shows(&rt, "Ctrl+K") && shows(&rt, "Maj+S"));
    assert_eq!(
        rt.state.shortcut_visible().len(),
        crate::shortcuts::ACTIONS.len()
    );
    rt.state.shortcuts.query = rune::TextEdit::new("ouvrir");
    step(&mut rt);
    let found: Vec<&str> = rt
        .state
        .shortcut_visible()
        .iter()
        .map(|a| a.label)
        .collect();
    assert_eq!(found, ["Ouvrir un dépôt"]);
    rt.state.shortcuts.query = rune::TextEdit::new("ctrl+r");
    assert_eq!(rt.state.shortcut_visible().len(), 1);
}

#[test]
fn a_customized_shortcut_works_replaces_the_old_key_persists_and_shows_in_the_hints() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "5");
    assert!(shows(&rt, "Nouvelle combinaison : 5."));
    assert!(shows(&rt, "5 · personnalisé"));
    let saved: BTreeMap<String, String> = rt.state.prefs.get("shortcuts").unwrap();
    assert_eq!(saved.get("view-staged").map(String::as_str), Some("5"));
    click(&mut rt, "sc-close");
    step(&mut rt);
    assert!(!rt.state.shortcuts.open);
    assert_eq!(rt.state.view, ViewKind::Unstaged);
    text(&mut rt, "2");
    assert_eq!(
        rt.state.view,
        ViewKind::Unstaged,
        "the old key no longer acts"
    );
    text(&mut rt, "5");
    assert_eq!(rt.state.view, ViewKind::Staged);
    assert!(shows(&rt, "1 et 5 : changer de vue"));
}

#[test]
fn conflicts_reserved_combinations_and_wrong_shapes_are_refused_before_anything_changes() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "1");
    assert!(shows(
        &rt,
        "déjà affectée à « Vue des fichiers non indexés »"
    ));
    assert!(rt.state.shortcuts.custom.is_empty());
    customize(&mut rt, "palette", "Ctrl+C");
    assert!(shows(&rt, "réservée au copier-coller"));
    customize(&mut rt, "palette", "q");
    assert!(shows(&rt, "demande Ctrl+une touche"));
    customize(&mut rt, "stage", "Ctrl+J");
    assert!(shows(&rt, "n'accepte qu'une touche seule"));
    customize(&mut rt, "refresh", "Ctrl+K");
    assert!(shows(&rt, "déjà affectée à « Palette de commandes »"));
    assert!(rt.state.shortcuts.custom.is_empty());
    assert!(
        rt.state
            .prefs
            .get::<BTreeMap<String, String>>("shortcuts")
            .is_none()
    );
}

#[test]
fn capture_can_be_cancelled_and_refuses_keys_that_cannot_be_assigned() {
    let (_dir, mut rt) = settings();
    rt.state.shortcut_select("mark");
    click(&mut rt, "sc-modify");
    step(&mut rt);
    press(&mut rt, Key::Escape, Mods::default());
    assert!(rt.state.shortcuts.capturing.is_none() && rt.state.shortcuts.open);
    click(&mut rt, "sc-modify");
    step(&mut rt);
    press(&mut rt, Key::Enter, Mods::default());
    assert!(shows(&rt, "ne peut pas être affectée"));
    assert!(rt.state.shortcuts.custom.is_empty());
    click(&mut rt, "sc-modify");
    step(&mut rt);
    ctrl(&mut rt, 'k');
    assert!(!rt.state.commands.open, "a captured key is never executed");
    assert!(shows(&rt, "n'accepte qu'une touche seule"));
}

#[test]
fn restoring_one_or_all_shortcuts_works_and_never_creates_a_conflict() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "5");
    customize(&mut rt, "view-unstaged", "2");
    rt.state.shortcut_select("view-staged");
    click(&mut rt, "sc-reset");
    step(&mut rt);
    assert!(shows(
        &rt,
        "Impossible de rétablir la combinaison par défaut"
    ));
    assert_eq!(
        rt.state
            .shortcuts
            .custom
            .get("view-staged")
            .map(String::as_str),
        Some("5")
    );
    rt.state.shortcut_select("view-unstaged");
    click(&mut rt, "sc-reset");
    step(&mut rt);
    assert!(shows(&rt, "Combinaison par défaut rétablie : 1."));
    rt.state.shortcut_select("view-staged");
    click(&mut rt, "sc-reset");
    step(&mut rt);
    assert!(rt.state.shortcuts.custom.is_empty());
    customize(&mut rt, "mark", "z");
    click(&mut rt, "sc-reset-all");
    step(&mut rt);
    assert!(rt.state.shortcuts.custom.is_empty());
    assert_eq!(
        rt.state.prefs.get::<BTreeMap<String, String>>("shortcuts"),
        Some(BTreeMap::new())
    );
}

#[test]
fn a_reading_shortcut_respects_the_input_focus_and_works_in_both_themes() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "5");
    click(&mut rt, "sc-close");
    step(&mut rt);
    for dark in [true, false] {
        rt.state.set_dark(dark);
        rt.state.set_view(ViewKind::Unstaged);
        step(&mut rt);
        text(&mut rt, "/");
        text(&mut rt, "5");
        assert_eq!(
            rt.state.view,
            ViewKind::Unstaged,
            "typing in the filter is not a shortcut"
        );
        assert_eq!(rt.state.filter.text, "5");
        press(&mut rt, Key::Escape, Mods::default());
        press(&mut rt, Key::Escape, Mods::default());
        rt.state.filter = rune::TextEdit::new("");
        step(&mut rt);
        text(&mut rt, "5");
        assert_eq!(rt.state.view, ViewKind::Staged, "dark={dark}");
    }
}

#[test]
fn a_global_shortcut_can_move_and_the_old_combination_stops_working() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "palette", "Ctrl+J");
    click(&mut rt, "sc-close");
    step(&mut rt);
    ctrl(&mut rt, 'k');
    assert!(!rt.state.commands.open);
    ctrl(&mut rt, 'j');
    assert!(rt.state.commands.open);
    assert!(
        shows(&rt, "Ctrl+J : palette de commandes") || rt.state.shortcut_for("palette") == "Ctrl+J"
    );
}

#[test]
fn saved_shortcuts_are_loaded_at_start() {
    let mut prefs = rune::prefs::Prefs::memory();
    let mut custom = BTreeMap::new();
    custom.insert("mark".to_owned(), "z".to_owned());
    prefs.set("shortcuts", &custom).unwrap();
    let state = State::new(None, prefs);
    assert_eq!(state.shortcut_for("mark"), "z");
    assert_eq!(state.shortcut_for("stage"), "s");
}

#[test]
fn the_palette_opens_the_settings() {
    let (_dir, mut rt) = super::keyboard_runtime(3);
    ctrl(&mut rt, 'k');
    for ch in "raccourcis".chars() {
        text(&mut rt, &ch.to_string());
    }
    press(&mut rt, Key::Enter, Mods::default());
    assert!(rt.state.shortcuts.open && !rt.state.commands.open);
}

#[test]
fn choosing_the_default_combination_again_leaves_no_custom_entry() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "5");
    assert_eq!(rt.state.shortcuts.custom.len(), 1);
    customize(&mut rt, "view-staged", "2");
    assert!(rt.state.shortcuts.custom.is_empty());
    assert!(!shows(&rt, "personnalisé"));
}

#[test]
fn reading_shortcuts_stay_silent_on_other_screens() {
    let (_dir, mut rt) = settings();
    customize(&mut rt, "view-staged", "5");
    rt.state.open_branches();
    step(&mut rt);
    text(&mut rt, "5");
    assert_eq!(rt.state.view, ViewKind::Unstaged);
}
