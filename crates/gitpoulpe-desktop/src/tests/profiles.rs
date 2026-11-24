use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gitpoulpe_git::SigningCheck;
use rune::prefs::Prefs;

use super::{click_at, pump, rect_of, runtime, step};
use crate::profiles::SigningKind;
use crate::state::{Screen, State};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    super::git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    super::git(dir.path(), &["add", "-A"]);
    super::git(dir.path(), &["commit", "-q", "-m", "base"]);
    dir
}

fn state_with(prefs: Prefs) -> State {
    let mut state = State::new(None, prefs);
    state.setup(Arc::new(|| {}));
    state
}

fn open(state: &mut State, dir: &Path) {
    state.open_blocking(dir.to_path_buf(), Duration::from_secs(20));
    assert_eq!(state.screen, Screen::Ready);
}

fn create(state: &mut State, label: &str, name: &str, email: &str) {
    state.new_profile();
    state.form.label.text = label.to_owned();
    state.form.name.text = name.to_owned();
    state.form.email.text = email.to_owned();
    state.save_profile();
    assert!(state.form.error.is_none(), "{:?}", state.form.error);
}

fn shown(state: &mut State) -> (Option<String>, String) {
    pump(state, |s| s.identity_shown.is_some());
    let shown = state.identity_shown.as_ref().unwrap();
    let identity = shown.result.clone().unwrap();
    (
        shown.profile.clone(),
        format!("{} <{}>", identity.name, identity.email),
    )
}

fn wait_identity(state: &mut State, expect: impl Fn(&State) -> bool) {
    pump(state, |s| expect(s));
}

#[test]
fn two_repositories_keep_two_profiles_and_the_assignments_survive_a_restart() {
    let prefs_dir = tempfile::tempdir().unwrap();
    let prefs_path: PathBuf = prefs_dir.path().join("prefs.json");
    let (a, b) = (repo(), repo());
    {
        let mut state = state_with(Prefs::at(&prefs_path));
        open(&mut state, a.path());
        create(&mut state, "Perso", "Ada Home", "ada@home.example");
        let perso = state.form.editing.clone().unwrap();
        create(&mut state, "Travail", "Ada Work", "ada@work.example");
        let travail = state.form.editing.clone().unwrap();

        state.use_profile_here(Some(perso));
        wait_identity(&mut state, |s| {
            s.identity_shown
                .as_ref()
                .is_some_and(|i| i.profile.as_deref() == Some("Perso"))
        });
        assert_eq!(shown(&mut state).1, "Ada Home <ada@home.example>");

        open(&mut state, b.path());
        state.use_profile_here(Some(travail));
        wait_identity(&mut state, |s| {
            s.identity_shown
                .as_ref()
                .is_some_and(|i| i.profile.as_deref() == Some("Travail"))
        });
        assert_eq!(shown(&mut state).1, "Ada Work <ada@work.example>");
        state.shutdown();
    }

    let mut again = state_with(Prefs::at(&prefs_path));
    open(&mut again, a.path());
    wait_identity(&mut again, |s| s.identity_shown.is_some());
    assert_eq!(shown(&mut again).1, "Ada Home <ada@home.example>");
    open(&mut again, b.path());
    wait_identity(&mut again, |s| {
        s.identity_shown
            .as_ref()
            .is_some_and(|i| i.profile.as_deref() == Some("Travail"))
    });
    assert_eq!(shown(&mut again).1, "Ada Work <ada@work.example>");
    again.shutdown();
}

fn bytes_of(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

#[test]
fn choosing_a_profile_never_writes_to_the_git_configuration() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let globals = [home.join(".gitconfig"), home.join(".config/git/config")];
    let before: Vec<_> = globals.iter().map(|p| bytes_of(p)).collect();
    let dir = repo();
    let local_before = std::fs::read(dir.path().join(".git/config")).unwrap();
    let mut state = state_with(Prefs::memory());
    open(&mut state, dir.path());
    create(&mut state, "Perso", "Ada", "ada@home.example");
    let id = state.form.editing.clone().unwrap();
    state.use_profile_here(Some(id));
    wait_identity(&mut state, |s| {
        s.identity_shown
            .as_ref()
            .is_some_and(|i| i.profile.as_deref() == Some("Perso"))
    });

    let after: Vec<_> = globals.iter().map(|p| bytes_of(p)).collect();
    assert_eq!(before, after, "the global Git configuration is untouched");
    assert_eq!(
        std::fs::read(dir.path().join(".git/config")).unwrap(),
        local_before,
        "the repository configuration is untouched"
    );
    state.shutdown();
}

#[test]
fn an_invalid_key_reference_explains_how_to_fix_it_and_no_key_material_is_stored() {
    let keys = tempfile::tempdir().unwrap();
    let private = keys.path().join("id_test");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&private)
        .status()
        .unwrap();
    assert!(status.success());
    let prefs_dir = tempfile::tempdir().unwrap();
    let prefs_path = prefs_dir.path().join("prefs.json");
    let dir = repo();
    let mut state = state_with(Prefs::at(&prefs_path));
    open(&mut state, dir.path());

    state.new_profile();
    state.form.label.text = "Signé".to_owned();
    state.form.name.text = "Ada".to_owned();
    state.form.email.text = "ada@example.org".to_owned();
    state.set_form_signing(SigningKind::Ssh);
    state.form.key.text = private.to_string_lossy().into_owned();
    state.save_profile();
    pump(&mut state, |s| s.form.signing_check.is_some());
    let Some(SigningCheck::Problem(message)) = state.form.signing_check.clone() else {
        panic!(
            "the private key must be refused: {:?}",
            state.form.signing_check
        )
    };
    assert!(
        message.contains("clé privée") && message.contains(".pub"),
        "{message}"
    );

    let stored = std::fs::read_to_string(&prefs_path).unwrap();
    assert!(
        stored.contains(&*private.to_string_lossy()),
        "only the reference is kept"
    );
    assert!(!stored.contains("BEGIN OPENSSH"), "no private key material");
    assert!(
        std::fs::read_to_string(&private)
            .unwrap()
            .starts_with("-----BEGIN"),
        "the key file is untouched"
    );

    state.form.key.text = format!("{}.pub", private.display());
    state.save_profile();
    pump(&mut state, |s| {
        s.form.signing_check == Some(SigningCheck::Ready)
    });
    state.shutdown();
}

#[test]
fn an_incomplete_profile_is_refused_with_a_reason_and_nothing_is_saved() {
    let prefs_dir = tempfile::tempdir().unwrap();
    let prefs_path = prefs_dir.path().join("prefs.json");
    let mut state = state_with(Prefs::at(&prefs_path));
    state.new_profile();
    state.form.label.text = "Sans e-mail".to_owned();
    state.form.name.text = "Ada".to_owned();
    state.form.email.text = "not-an-email".to_owned();
    state.save_profile();
    assert_eq!(
        state.form.error.as_deref(),
        Some("Indique une adresse e-mail valide (user.email).")
    );
    assert!(state.book.profiles.is_empty());
    assert!(
        !prefs_path.exists()
            || !std::fs::read_to_string(&prefs_path)
                .unwrap()
                .contains("Sans e-mail")
    );
    state.shutdown();
}

#[test]
fn deleting_the_profile_of_a_repository_makes_it_follow_the_git_configuration_again() {
    let dir = repo();
    let mut state = state_with(Prefs::memory());
    open(&mut state, dir.path());
    create(&mut state, "Perso", "Ada", "ada@home.example");
    let id = state.form.editing.clone().unwrap();
    state.use_profile_here(Some(id));
    wait_identity(&mut state, |s| {
        s.identity_shown
            .as_ref()
            .is_some_and(|i| i.profile.as_deref() == Some("Perso"))
    });

    state.delete_profile();
    assert!(state.book.assignments.is_empty());
    wait_identity(&mut state, |s| {
        s.identity_shown
            .as_ref()
            .is_some_and(|i| i.profile.is_none())
    });
    state.shutdown();
}

#[test]
fn the_profile_screen_opens_from_the_menu_saves_from_its_button_and_closes_with_escape() {
    let dir = repo();
    let mut state = state_with(Prefs::memory());
    open(&mut state, dir.path());
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    let button = rect_of(&rt, "profiles").expect("the menu offers the profiles");
    click_at(
        &mut rt,
        button[0] + 10.0,
        button[1] + button[3] / 2.0,
        false,
    );
    for _ in 0..4 {
        step(&mut rt);
    }
    assert!(rt.state.profiles_open);
    for key in ["pf-label", "pf-name", "pf-email", "pf-save", "pf-close"] {
        assert!(rect_of(&rt, key).is_some(), "{key} is missing");
    }

    rt.state.form.label.text = "Perso".to_owned();
    rt.state.form.name.text = "Ada".to_owned();
    rt.state.form.email.text = "ada@home.example".to_owned();
    for _ in 0..2 {
        step(&mut rt);
    }
    let save = rect_of(&rt, "pf-save").unwrap();
    click_at(&mut rt, save[0] + 10.0, save[1] + save[3] / 2.0, false);
    for _ in 0..4 {
        step(&mut rt);
    }
    assert_eq!(rt.state.book.profiles.len(), 1);

    super::press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.profiles_open);
    rt.state.shutdown();
}
