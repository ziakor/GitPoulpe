use std::path::{Path, PathBuf};
use std::process::Command;

use gitpoulpe_core::{ToolBook, ToolConfig};
use rune::{Runtime, TextEdit};

use super::{click_at, rect_of, runtime, step};
use crate::state::State;

const SCRIPT: &str = "log=\"$1\"; code=\"$2\"; shift 2\n: > \"$log\"\nfor a in \"$@\"; do\n  last=\"$a\"\n  printf 'ARG<%s>\\n' \"$a\" >> \"$log\"\n  if [ -f \"$a\" ]; then printf 'CONTENT<%s>\\n' \"$(cat \"$a\")\" >> \"$log\"; fi\ndone\nexit \"$code\"\n";
const APPENDING: &str = "log=\"$1\"; code=\"$2\"; shift 2\n: > \"$log\"\nfor a in \"$@\"; do last=\"$a\"; printf 'ARG<%s>\\n' \"$a\" >> \"$log\"; done\nprintf 'tool line\\n' >> \"$last\"\nexit \"$code\"\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..400 {
        step(rt);
        if until(&rt.state) {
            for _ in 0..8 {
                step(rt);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

struct Fixture {
    repo: tempfile::TempDir,
    tool: tempfile::TempDir,
    log: PathBuf,
}

const NAME: &str = "we ird 'q\" $(x);*é.txt";

fn fixture() -> Fixture {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join(NAME), "version one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "one"]);
    std::fs::write(p.join(NAME), "version two\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "two"]);
    std::fs::write(p.join(NAME), "version two\nlocal\n").unwrap();
    let tool = tempfile::tempdir().unwrap();
    std::fs::write(tool.path().join("tool.sh"), SCRIPT).unwrap();
    std::fs::write(tool.path().join("append.sh"), APPENDING).unwrap();
    let log = tool.path().join("log.txt");
    Fixture { repo, tool, log }
}

impl Fixture {
    fn config(&self, script: &str, code: i32, template: &str) -> ToolConfig {
        ToolConfig {
            program: "sh".to_owned(),
            args: format!(
                "'{}' '{}' {code} {template}",
                self.tool.path().join(script).display(),
                self.log.display()
            ),
        }
    }

    fn recorded(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn app(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn finished(rt: &mut Runtime<State>) {
    settle(rt, |s| s.tools.running.is_none() && s.tools.request > 0);
}

#[test]
fn the_editor_receives_the_exact_path_and_the_screen_reloads_what_it_changed() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.tools.book.editor = f.config("append.sh", 0, "{file}");
    rt.state.open_file_edit(NAME.to_owned());
    settle(&mut rt, |s| s.file_edit.original.is_some());
    click(&mut rt, "fe-external");
    finished(&mut rt);
    let expected = f.repo.path().join(NAME);
    assert!(
        f.recorded()
            .contains(&format!("ARG<{}>", expected.display()))
    );
    assert!(shows(&rt, "Éditeur : session terminée."));
    settle(&mut rt, |s| s.file_edit.text.text.contains("tool line"));
    assert!(!rt.state.file_edit.dirty());
}

#[test]
fn unsaved_text_is_never_hidden_behind_an_external_session() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.tools.book.editor = f.config("tool.sh", 0, "{file}");
    rt.state.open_file_edit(NAME.to_owned());
    settle(&mut rt, |s| s.file_edit.original.is_some());
    rt.state.file_edit.text = TextEdit::new("typed\n");
    step(&mut rt);
    click(&mut rt, "fe-external");
    step(&mut rt);
    assert!(shows(&rt, "Enregistrez d'abord"));
    assert!(rt.state.tools.running.is_none());
    assert_eq!(f.recorded(), "");
    assert_eq!(rt.state.file_edit.text.text, "typed\n");
}

#[test]
fn a_missing_or_failing_tool_gives_a_recoverable_error_and_a_later_run_works() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.open_file_edit(NAME.to_owned());
    settle(&mut rt, |s| s.file_edit.original.is_some());
    click(&mut rt, "fe-external");
    step(&mut rt);
    assert!(shows(&rt, "Éditeur non configuré"));
    rt.state.tools.book.editor = ToolConfig {
        program: "definitely-not-installed-xyz".to_owned(),
        args: "{file}".to_owned(),
    };
    click(&mut rt, "fe-external");
    finished(&mut rt);
    assert!(shows(
        &rt,
        "exécutable introuvable : definitely-not-installed-xyz"
    ));
    rt.state.tools.book.editor = f.config("tool.sh", 3, "{file}");
    click(&mut rt, "fe-external");
    finished(&mut rt);
    assert!(shows(&rt, "code 3"));
    assert!(rt.state.tools.running.is_none());
    rt.state.tools.book.editor = f.config("tool.sh", 0, "{file}");
    click(&mut rt, "fe-external");
    finished(&mut rt);
    assert!(shows(&rt, "session terminée"));
}

#[test]
fn the_comparison_tool_gets_both_versions_of_the_selected_revision_and_its_temporaries_disappear() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.tools.book.diff = f.config("tool.sh", 0, "{left} {right}");
    rt.state.open_file_history(NAME.to_owned());
    settle(&mut rt, |s| s.file_history.history.is_some());
    let oid = rt
        .state
        .file_history
        .history
        .as_ref()
        .unwrap()
        .revisions
        .iter()
        .find(|r| r.subject == "two")
        .unwrap()
        .oid
        .clone();
    rt.state.select_file_revision(oid);
    settle(&mut rt, |s| !s.file_history.revision_loading);
    click(&mut rt, "fh-external-diff");
    finished(&mut rt);
    let log = f.recorded();
    assert!(
        log.contains("CONTENT<version one>") && log.contains("CONTENT<version two>"),
        "{log}"
    );
    let paths: Vec<&str> = log
        .lines()
        .filter_map(|l| l.strip_prefix("ARG<").and_then(|l| l.strip_suffix('>')))
        .collect();
    assert_eq!(paths.len(), 2);
    assert!(
        paths
            .iter()
            .all(|p| p.ends_with(NAME) && !Path::new(p).exists())
    );
    assert!(shows(&rt, "Outil de comparaison : session terminée."));
}

#[test]
fn with_no_revision_selected_the_working_copy_is_compared_with_the_last_commit() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.tools.book.diff = f.config("tool.sh", 0, "{left} {right}");
    rt.state.open_file_history(NAME.to_owned());
    settle(&mut rt, |s| s.file_history.history.is_some());
    click(&mut rt, "fh-external-diff");
    finished(&mut rt);
    let log = f.recorded();
    assert!(log.contains("CONTENT<version two>"));
    assert!(log.contains(&format!("ARG<{}>", f.repo.path().join(NAME).display())));
    assert!(log.contains("CONTENT<version two\nlocal>"));
}

#[test]
fn the_merge_tool_gets_base_ours_and_theirs_and_the_conflict_list_is_refreshed_afterwards() {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "line1\nline2\nline3\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "line1\nfeature\nline3\n").unwrap();
    git(p, &["commit", "-qam", "feature work"]);
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("a.txt"), "line1\nmain\nline3\n").unwrap();
    git(p, &["commit", "-qam", "main work"]);
    git(p, &["merge", "feature"]);
    let tool = tempfile::tempdir().unwrap();
    std::fs::write(tool.path().join("tool.sh"), SCRIPT).unwrap();
    let log = tool.path().join("log.txt");
    let mut rt = app(p);
    settle(&mut rt, |s| s.operation_kind.is_some());
    click(&mut rt, "conflicts-open");
    settle(&mut rt, |s| {
        s.conflicts.open && !s.conflicts.loading && s.conflicts.versions.is_some()
    });
    rt.state.tools.book.merge = ToolConfig {
        program: "sh".to_owned(),
        args: format!(
            "'{}' '{}' 0 {{base}} {{ours}} {{theirs}} {{merged}}",
            tool.path().join("tool.sh").display(),
            log.display()
        ),
    };
    rt.state.select_conflict(PathBuf::from("a.txt"));
    settle(&mut rt, |s| {
        !s.conflicts.loading && s.conflicts.versions.is_some()
    });
    let before = rt.state.conflicts.request;
    click(&mut rt, "cf-external-merge");
    finished(&mut rt);
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("CONTENT<line1\nline2\nline3>"));
    assert!(recorded.contains("CONTENT<line1\nmain\nline3>"));
    assert!(recorded.contains("CONTENT<line1\nfeature\nline3>"));
    assert!(recorded.contains(&format!("ARG<{}>", p.join("a.txt").display())));
    assert!(shows(&rt, "Outil de fusion : session terminée."));
    assert!(
        rt.state.conflicts.request > before,
        "the conflict list was read again"
    );
}

#[test]
fn the_configuration_is_validated_saved_and_keeps_the_underlying_screen_intact() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.open_file_edit(NAME.to_owned());
    settle(&mut rt, |s| s.file_edit.original.is_some());
    rt.state.file_edit.text = TextEdit::new("typed\n");
    step(&mut rt);
    click(&mut rt, "fe-tools");
    step(&mut rt);
    assert!(rt.state.tools.open && shows(&rt, "Outils externes"));
    rt.state.tools.editor.program = TextEdit::new("code");
    rt.state.tools.editor.args = TextEdit::new("--wait {nope}");
    step(&mut rt);
    click(&mut rt, "tl-save");
    step(&mut rt);
    assert!(shows(&rt, "variable inconnue dans les arguments : {nope}"));
    assert!(!rt.state.tools.book.editor.configured());
    rt.state.tools.editor.args = TextEdit::new("--wait \"{file}");
    step(&mut rt);
    click(&mut rt, "tl-save");
    step(&mut rt);
    assert!(shows(&rt, "guillemet non fermé"));
    rt.state.tools.editor.args = TextEdit::new("--wait {file}");
    step(&mut rt);
    click(&mut rt, "tl-save");
    step(&mut rt);
    assert!(shows(&rt, "Outils enregistrés."));
    assert_eq!(
        rt.state
            .prefs
            .get::<ToolBook>("external_tools")
            .unwrap()
            .editor
            .program,
        "code"
    );
    click(&mut rt, "tl-close");
    step(&mut rt);
    assert!(!rt.state.tools.open && rt.state.file_edit.open);
    assert_eq!(rt.state.file_edit.text.text, "typed\n");
}

#[test]
fn a_second_launch_while_a_session_runs_and_a_late_answer_are_ignored() {
    let f = fixture();
    let mut rt = app(f.repo.path());
    rt.state.tools.book.editor = f.config("tool.sh", 0, "{file}");
    rt.state.open_file_edit(NAME.to_owned());
    settle(&mut rt, |s| s.file_edit.original.is_some());
    rt.state.tool_open_editor(NAME.to_owned());
    rt.state.tool_open_editor(NAME.to_owned());
    assert_eq!(rt.state.tools.request, 1);
    assert!(!rt.state.apply_tool_finished(0, Err("late".to_owned())));
    assert!(rt.state.tools.running.is_some());
    finished(&mut rt);
    assert!(!rt.state.apply_tool_finished(1, Err("duplicate".to_owned())));
    assert!(rt.state.tools.status_error.is_none());
}

#[test]
fn a_renamed_file_is_compared_with_its_content_under_the_old_name() {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "v1\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "first"]);
    git(p, &["mv", "a.txt", "b.txt"]);
    std::fs::write(p.join("b.txt"), "v1\nv2\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "rename"]);
    let tool = tempfile::tempdir().unwrap();
    std::fs::write(tool.path().join("tool.sh"), SCRIPT).unwrap();
    let log = tool.path().join("log.txt");
    let mut rt = app(p);
    rt.state.tools.book.diff = ToolConfig {
        program: "sh".to_owned(),
        args: format!(
            "'{}' '{}' 0 {{left}} {{right}}",
            tool.path().join("tool.sh").display(),
            log.display()
        ),
    };
    rt.state.open_file_history("b.txt".to_owned());
    settle(&mut rt, |s| s.file_history.history.is_some());
    let oid = rt
        .state
        .file_history
        .history
        .as_ref()
        .unwrap()
        .revisions
        .iter()
        .find(|r| r.subject == "rename")
        .unwrap()
        .oid
        .clone();
    rt.state.select_file_revision(oid);
    settle(&mut rt, |s| !s.file_history.revision_loading);
    click(&mut rt, "fh-external-diff");
    finished(&mut rt);
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("CONTENT<v1>"), "{recorded}");
    assert!(recorded.contains("CONTENT<v1\nv2>"), "{recorded}");
}
