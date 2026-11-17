# GitPoulpe

A free, open-source native Git client written in Rust, with the [Rune](https://github.com/anomalyco/rune) UI toolkit (GPLv3). Linux first; Windows and macOS will follow.

Status: milestone M1, "local inspection", is delivered. Milestone M2, "local work", is in progress: so far the app can stage whole files, single blocks and selected lines. Reading a repository never modifies it; the only writes are the explicit actions listed under "Writing".

The interface is currently in French only.

## What M1 does

- Opens a repository from the folder picker ("Ouvrir un dépôt…" button or Ctrl+O) or from a path given as an argument. Ordinary repositories, repositories without a first commit and linked worktrees work. A bare repository is reported as having no working directory.
- Two views, unstaged and staged. A file changed in both appears in each, with its own diff.
- All diffs scroll in a single stream, in unified or split mode, with selection and copy. Binary files, conflicts and unreadable files are announced where they sit in the stream.
- Filter by name or path.
- Refreshes when the repository changes, when the window regains focus, or on demand (Refresh button, `r` key). The reading position is kept.

Git runs as an external program, without a shell, with the repository's filters, fsmonitor programs and network access disabled.

## Writing

- Stage files (unstaged view): "Indexer" button in a file header, `s` for the current file, or tick several files (checkbox or `x`) and use "Indexer la sélection" or `Shift+S`.
- Stage a single block of a diff: "Indexer ce bloc" button on the block header (unified and split modes). Only for tracked text files with content changes; untracked, deleted, renamed and mode-changed files are staged whole, and block staging is disabled in repositories that configure Git filters.
- Stage selected lines: click a line number in the gutter to select an added or removed line, Shift+click another one to select everything in between (the selection can go far outside the visible area), then use the "Indexer les lignes" button in the file header or `s`. `Escape` clears the selection. An unselected removed line stays in the index and an unselected added line is left out, as with `git add -p`. A partial selection in a block that touches a file's missing final newline is refused; select the whole block instead.
- Nothing is staged if a file changed after it was displayed, if the index changed, or if the file is in conflict; the state is re-read and a message says why.
- Git runs as configured for these actions (clean filters, such as Git LFS, apply). An existing `index.lock` is reported, never removed.

## Keyboard shortcuts

| Key | Action |
| -- | -- |
| `/` | filter |
| `j` / `k` | next / previous file |
| `1` / `2` | unstaged / staged view |
| `m` | unified / split mode |
| `r` | refresh |
| `s` / `x` / `Shift+S` | stage the selected lines, or else the current file / tick the file / stage the ticked files |
| arrows, PgUp, PgDn, Home, End | read |
| Ctrl+O | open a repository |

## Build and run

Requirements: Rust 1.100.0 (`rustup toolchain install 1.100.0`), Git on the `PATH` (tested with 2.53.0), a GPU and a Linux graphical session. The folder picker uses the XDG desktop portal.

```sh
cargo +1.100.0 build --release -p gitpoulpe-desktop
target/release/gitpoulpe /path/to/repository
```

Without an argument, the app opens on its welcome screen.

## Tests

```sh
cargo +1.100.0 test --workspace
cargo +1.100.0 clippy --workspace --all-targets -- -D warnings
```

## Known limits

- Tested on Linux only.
- Performance (CV-05) is accepted with a reservation. In a real window, a pause of about 90 to 100 ms roughly every 10 s is unexplained. Rune provides no presentation timestamps, so no headless result proves 60 frames per second.
- A change to `.gitignore` during a session is only picked up by the watcher when the repository is reopened.

## Layout

Three crates: `gitpoulpe-core` (types and contracts), `gitpoulpe-git` (reading status and diffs, cache, watching) and `gitpoulpe-desktop` (the UI).

## License

GNU General Public License v3.0 only. See [LICENSE](LICENSE).
