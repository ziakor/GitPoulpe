# GitPoulpe

A free, open-source native Git client written in Rust, with the [Rune](https://github.com/anomalyco/rune) UI toolkit (GPLv3). Linux first; Windows and macOS will follow.

Status: milestone M1, "local inspection", is delivered. Milestone M2, "local work", is in progress: so far the app can stage and unstage whole files, single blocks and selected lines, manage local Git profiles and create commits. Reading a repository never modifies it; the only writes are the explicit actions listed under "Writing".

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
- Unstage, in the staged view: "Désindexer" button in a file header, `u` for the current file (or for the selected lines when there are some), tick several files and use "Désindexer la sélection" or `Shift+U`. "Désindexer ce bloc" and line selection work like their staging counterparts, including for newly added files; deleted, renamed and mode-changed files are unstaged whole. A staged rename is undone as a whole. Unstaging never touches the working directory and works before the first commit.
- Nothing is staged if a file changed after it was displayed, if the index changed, or if the file is in conflict; the state is re-read and a message says why.
- Git runs as configured for these actions (clean filters, such as Git LFS, apply). An existing `index.lock` is reported, never removed.

## History

- "Historique" button in the top bar: the commits of all branches, tags and remote-tracking branches plus HEAD, in Git's topological order, with a lane graph (branches, merges, octopus merges), references, author and UTC date. Only the first 200 commits are read; scrolling near the end reads the next page, so a very long history is never loaded at once and rendering stays bounded to what is on screen.
- Click a commit to see its details; selecting never blocks scrolling. `Escape` closes the history. The list reloads when the repository's current commit changes (for example after you commit).
- Search: the box above the history looks for a commit by message (subject and body), author or the beginning of its identifier (four hexadecimal digits at least; every commit sharing the prefix is listed, none is guessed). Matching ignores case. Results are read progressively in history order, a newer query cancels the previous one and its late answers are discarded, and "searching", "no result" and "error" are different states. Choosing a result opens that commit in the history, loading the pages in between if needed.
- Inspect a commit: select it and use "Inspecter ce commit". The commit view shows the message, author and date, the files the commit changed (renames detected) and their diffs in the same continuous reader as the working changes, with binary files and unusual paths handled as everywhere else. For a merge, choose which parent to compare against; the first commit is compared with an empty tree. No staging, unstaging or commit control is offered there, and `Escape` or "Quitter l'inspection" returns to the history. Commit ids passed to Git are validated as hexadecimal and the comparison is read-only.
- A repository without any commit and a Git failure are different states, so a failure never looks like an empty history.

## Committing

- Write a message in the box at the bottom of the file list, then "Créer le commit" or Ctrl+Enter. The commit contains exactly the staged content shown, with the effective identity of the repository's profile (or your Git configuration when it has none). The box also shows who will be the author and whether the profile's signing key is usable.
- Nothing is committed when the message is empty, nothing is staged, conflicts are pending, or the index changed after it was displayed. The message you typed is kept in every failure case.
- Git runs as configured: hooks (`pre-commit`, `commit-msg`) are executed. If a hook refuses, its output is shown under the message and you can fix the problem and retry. Unknown identity and signing failures are reported in the same way, before anything is written when possible. An existing `index.lock` is reported, never removed.
- Amend the last commit: tick "Amender le dernier commit" above the message. The last message is offered (a draft you typed is never overwritten), staged changes are folded into the commit, and the original author is kept while the committer is your profile. If the commit is already published on a remote-tracking branch, the consequences are shown before anything happens and a second explicit click is required. Nothing is amended when nothing would change, when the tip moved since it was displayed, or when a hook or the signature refuses; the previous commit stays reachable and its id is shown and recorded.
- Each commit is recorded in a per-repository journal (50 entries, in GitPoulpe's preferences, outside the repository) with the commit before and after, as a recovery point.

## Profiles

- "Profil" button in the top bar: create profiles (name, Git name, e-mail, optional commit signing reference) and choose which one a repository uses. The effective identity (what Git would use, from `git var`) is shown for the open repository.
- A profile is applied only to the commands GitPoulpe runs. Nothing is written to your Git configuration, global or local; a commit made in a terminal can therefore use another identity.
- Profiles and the repository assignments are stored in GitPoulpe's own preferences file, outside the repository.
- For signing, only a reference is stored: the path of an SSH public key (`.pub`) or a GPG key identifier. A private key path is refused, and an unreadable key or a GPG key missing from the keyring is reported with how to fix it. No private key or passphrase is ever read or stored.
- GPG signing: choose "GPG" and the key fingerprint. The passphrase prompt is the one of your `gpg-agent` / pinentry; GitPoulpe never sees, stores or logs a passphrase. A cancelled prompt, a locked key or a key missing from the keyring creates no commit and keeps the work staged; after unlocking you can retry the same commit. A graphical pinentry is needed (a terminal-only pinentry cannot prompt from a window application).
- SSH signing: choose "SSH" and the path of your public key. Git signs through `ssh-agent` (or the matching private key file, as OpenSSH does); the commit box states the signing mode and whether the key is usable before you commit, and after a signed commit GitPoulpe checks the signature against the profile's key. If neither an agent nor the private key is available, nothing is committed and the diagnostic is kept.

## Keyboard shortcuts

| Key | Action |
| -- | -- |
| `/` | filter |
| `j` / `k` | next / previous file |
| `1` / `2` | unstaged / staged view |
| `m` | unified / split mode |
| `r` | refresh |
| `s` / `u` | stage / unstage the selected lines, or else the current file |
| `x` / `Shift+S` or `Shift+U` | tick the current file / apply to the ticked files |
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
