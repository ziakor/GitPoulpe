# Contributing to GitPoulpe

GitPoulpe is free software under the GNU General Public License v3.0 only. You do not need an account on any GitPoulpe service to contribute: everything below works with Git, a toolchain and an email client or any Git host you already use.

## Environment

- Linux with a graphical session and a GPU (the tests themselves run headless).
- Rust 1.100.0: `rustup toolchain install 1.100.0` (the workspace pins `rust-version = "1.100.0"`, edition 2024).
- Git on the `PATH` (developed against 2.53.0) and Python 3 (the hosting tests start small fake GitHub and GitLab servers written in Python).
- Network access for the first build: the Rune UI toolkit is a Git dependency pinned to an exact revision in `Cargo.toml`, the other dependencies come from crates.io at exact versions.

## Build and checks

```sh
git clone <your fork or the upstream repository> gitpoulpe && cd gitpoulpe
cargo +1.100.0 build --workspace --locked
cargo +1.100.0 fmt --all -- --check
cargo +1.100.0 clippy --workspace --all-targets --locked -- -D warnings
cargo +1.100.0 test --workspace --locked
python3 scripts/check-licenses.py
```

Run the whole test suite once before proposing a change; while working, run only the tests your change touches (`cargo +1.100.0 test -p gitpoulpe-desktop <name>`). The desktop tests drive the real UI runtime and take a few minutes together. Read the clippy output: a clean exit is the bar, not a skimmed log.

`packaging/linux/test.sh` rebuilds the Linux package and exercises install and uninstall in temporary directories. It needs a release build (several minutes) and is only required for packaging changes. The reference-repository performance check is opt-in: `GITPOULPE_REFERENCE_REPO=/path/to/large/repo cargo +1.100.0 test --release -p gitpoulpe-desktop reference_repository_cpu_budget -- --ignored --nocapture`. Include its numbers when you change rendering, diff loading or watching.

## Layout

| Crate | Role |
| -- | -- |
| `gitpoulpe-core` | Plain types and contracts shared by the others (no I/O, serde only). |
| `gitpoulpe-git` | Everything that runs `git`: status, diff, history, operations, patches, watching, cache. |
| `gitpoulpe-hosting` | GitHub and GitLab REST clients: accounts, pull requests, checks, issues, notifications. |
| `gitpoulpe-terminal` | The embedded terminal (pseudo-terminal and screen state). |
| `gitpoulpe-desktop` | The application: `State` plus a `view` function on the Rune toolkit, one `<feature>.rs` and `<feature>_view.rs` pair per screen, background `engine.rs`, tests under `src/tests/`. |

New work goes in the crate that owns the concern; the UI never shells out to Git itself.

## Conventions

- Code comments are in English and rare: write one only for a constraint the code cannot express, one line when possible, nothing you have not verified.
- User-facing strings are in French. README and this file are in English.
- Invariant parameters are constants in code. Add an environment variable only for a secret, a real per-environment setting or a proven operational control.
- Dependencies are pinned with `=`. Before adding or bumping one, check the published version (`cargo search`, `cargo info`) and its licence; `scripts/check-licenses.py` must still pass. A new bundled asset (font, icon, data file) needs its licence text next to it and an entry in `gitpoulpe --licenses`.
- Tests use real temporary Git repositories and the fake provider servers in `crates/gitpoulpe-hosting/tests/common/`, not mocks of Git. A bug fix starts with a test that fails for the right reason. For risky logic, break the code on purpose and confirm a test fails before you restore it.
- Never swallow a technical failure behind a friendly message: keep the cause, filter secrets and customer data before logging.
- Keep commits small and each one working.

## Proposing a change

1. Fork the repository or work on a local clone; open an issue first for anything large so the approach can be agreed.
2. Create a branch, make the change with its tests and, if behaviour is visible to users, a short README section in the same style as the existing ones.
3. Run the checks above. Say in the proposal what you ran and what you could not run (other operating systems, real provider accounts, a real GPU): do not claim verification you did not do.
4. Send it as a pull or merge request on the host of your choice, or as patches: `git format-patch origin/main` and email them or attach them to an issue. Maintainers can apply them with the app's "Importer un patch…" command or `git am`.
5. By submitting, you agree that your contribution is released under GPL-3.0-only.

## Provenance

The binary, dependencies and embedded fonts are listed with their licences by `gitpoulpe --licenses`, and a release package carries `PROVENANCE.txt` (source commit, compiler, SHA-256 of the binary, `Cargo.lock` and each font). `python3 scripts/check-licenses.py --list` prints the licence of every third-party crate.
