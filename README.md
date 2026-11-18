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

## Ignored files

- The "Ignorés" screen lists the paths Git ignores (a fully ignored directory is listed once, the list is capped at 5000 entries). Selecting one shows the rule that applies, its line and its scope: a `.gitignore` (root or nested), `.git/info/exclude` or the user's global exclude file. An exception (`!pattern`) is reported as a re-inclusion, not as an exclusion.
- A file already tracked by Git is reported as tracked: an ignore rule never removes it from the index, and GitPoulpe never runs `git rm --cached` for you.
- Rules are added or removed in one file that you choose (the root `.gitignore`, the one of the file's directory, `.git/info/exclude` or the global file). Only that file is rewritten, atomically (temporary file then rename); the rewrite is abandoned if the file changed in between, a symlinked `.gitignore` or `.git/info/exclude` is refused (never written through), and the global file is taken from the user-level Git configuration only, never from the repository's own `core.excludesFile`. Rules built from a path are anchored and escaped, so they match that path only.
- Untracked files have an "Ignorer…" button on their header that opens the same screen on that file.
- Paths that are not valid UTF-8 cannot be turned into a rule automatically.

## Branches

- Click the branch name in the top bar to manage local branches: create one from `HEAD`, switch to one, rename one, delete one. Names are validated with `git check-ref-format` before any Git call, and a branch name can never be read as an option.
- Switching uses `git switch --no-guess`: Git itself refuses when a local change or an untracked file would be overwritten, and its message is shown. Compatible local changes travel with the switch, as with Git.
- A branch checked out in another worktree is shown with that worktree's path and offers neither a switch nor a deletion.
- Deleting a branch that is fully merged is immediate. Deleting one that is not merged first lists the commits no other branch reaches (up to 100) and the command to bring the branch back; only then can it be forced, and only if the branch still points at the commit that was listed. The tip of every deleted branch is also written to the per-repository journal.
- Remote-tracking branches and tags are not touched by this screen.

## Tags

- The "Tags" button of the branch screen (or "Créer un tag ici" on a commit in the history) opens the tag screen. It lists local tags, light or annotated, with their commit; an annotated tag also shows its tagger, date and full message, and "Inspecter le commit" opens the tagged commit.
- A light tag or an annotated tag (message required) is created on `HEAD` or on the commit chosen in the history. The name is validated with `git check-ref-format` first; an existing name is refused and a tag is never replaced or moved. An annotated tag is signed with the active profile's identity, or the configured one, and creation stops when no identity is known.
- Deleting a tag is explicit and asks first. It names the tag, states that no branch is touched, and gives the `git update-ref` command that restores it, annotation included. It only goes through if the tag still points at the object that was displayed. Deletions and creations are written to the per-repository journal.
- Remote tags are not read or pushed.

## Initialising a repository

- On the welcome screen, "Initialiser un dossier…" asks for a folder; when opening a folder fails because it is not a repository, "Initialiser ce dossier" starts from that folder. A panel shows the path and the initial branch (the `init.defaultBranch` setting, else `main`), validated with `git check-ref-format`.
- The folder becomes an empty repository without a first commit and opens immediately; the files already in it appear as untracked.
- An existing repository, including a bare one, is never reinitialised, and a folder inside another repository is refused with the path of the enclosing one. A refused access or an interrupted creation leaves the panel open with the reason; trying again completes a half-created `.git`.

## Remotes

- The "Remotes" button of the branch screen opens the remote screen: each remote with its read (fetch) and write (push) URLs, as Git resolves them. A separate push URL is flagged.
- Adding a remote validates the name (`git check-ref-format`) and the URL; an existing name is refused and its configuration is never replaced. Accepted URLs are `https`, `http`, `ssh`, `git`, `file`, `host:path` and local paths. Transport helpers (`ext::`, `fd::`), options disguised as hosts (a leading `-`), blanks and control characters are refused, since they can run commands.
- Changing the read URL, the push URL or the name only goes through if the remote still has the URLs that were displayed, and never rewrites a remote that has several URLs (edit those with `git config`). Renaming moves the remote-tracking branches and the branches' upstream setting with it. Everything applies to the open repository only.
- Removing a remote first states its impact: how many remote-tracking branches go away and which local branches lose their upstream. Local branches and commits are kept. The `git remote add` command that restores it is shown.
- URLs that can carry a secret are shown masked, never prefilled in a field and never repeated in a recovery command: any user part on `http`, `https` or `git` (a token is often used as the user name), a password on `ssh://`, and any query string or fragment. The new URL has to be typed in full.
- Changes are written to the per-repository journal, with credentials masked.

## Access over SSH

- "Tester l'accès" on a remote runs a read (`git ls-remote`) with your existing SSH setup and reports the branches, tags and `HEAD` it found. The operation can be cancelled; Git and every process it started (ssh, helpers) are stopped together.
- GitPoulpe uses your `ssh` configuration, agent and keys as they are. It adds only `BatchMode=yes` and connection time-outs to the SSH command (your own `GIT_SSH_COMMAND` or global `core.sshCommand` is the base); a key protected by a passphrase therefore has to be loaded in the agent (`ssh-add`). Host verification is never disabled.
- An unknown host is a decision: the fingerprints it presents are read with `ssh-keyscan`, shown, and written to `known_hosts` only when you accept one of them. The key is scanned again at that moment and must still match, and only a plain `host key` line for that exact host is accepted. A host that is already known but presents another key is refused with no way to accept it from GitPoulpe; the `ssh-keygen -R` command is shown for you to run if the change is legitimate.
- A refused access says whether an SSH agent was reachable. Private keys are never read, copied, stored or logged by GitPoulpe, and an SSH command written in a repository's own configuration (`core.sshCommand`) is ignored for network operations.
- Network operations only allow the `https`, `http`, `ssh`, `git` and `file` transports, and never the `ext::` helpers.

## Access over HTTPS

- The same "Tester l'accès" works for `https` remotes. Credentials come from your own Git setup: `credential.helper`, `core.askPass` or `GIT_ASKPASS` from the system and global configuration. GitPoulpe stores nothing and never puts a secret in a URL; when no mechanism provides credentials, or the server refuses them, the operation ends with a message pointing at `git config --global credential.helper`. Terminal prompts are disabled.
- A `credential.helper` or `core.askPass` written in a repository's own configuration is ignored for network operations, because it would run a program chosen by whoever made the repository.
- Cancelling stops Git and the helper or prompt it started.
- Certificate checks are always on: `http.sslVerify=false` (in any configuration) and `GIT_SSL_NO_VERIFY` are overridden. An untrusted certificate is reported as a TLS error; to accept a private authority, point `http.sslCAInfo` at its certificate in your global configuration.
- Network and name-resolution failures, authentication refusals and missing repositories are reported separately, with any credential embedded in a URL hidden from the message.

## Fetching

- "Récupérer (fetch)" on a remote updates its remote-tracking branches and tags (`git fetch`), with the progress of the transfer, a cancel button and a summary of what changed. It works over SSH and HTTPS as described above, and the remote-tracking branches then show in the history graph.
- Fetching never touches your working tree, your index or your local branches. An interrupted, cancelled or failed fetch leaves the known references as they were (Git updates them only once the objects have arrived) and can simply be launched again. Submodules are not fetched and Git's automatic maintenance is not started.

## Cloning

- "Cloner un dépôt…" on the welcome screen: the URL, a destination folder (chosen with the folder dialog) and the folder name (proposed from the URL). The transfer shows its progress and can be cancelled, then the cloned repository opens on the branch the remote designates.
- The clone is made in a hidden temporary folder next to the destination and moved into place only when it succeeded: a failure or a cancellation (which stops Git and the processes it started) leaves no partial folder, and the same form can be submitted again. An existing destination is accepted only when it is an empty folder; a non-empty one is refused before any network access and left untouched.
- A URL carrying credentials is refused, since Git would write them to the repository configuration; authentication goes through the SSH and HTTPS mechanisms described above. The URL rules of the remote screen apply (no `ext::` helpers, no option disguised as a host).

## Resolving conflicts

- When a merge, rebase, cherry-pick or revert is in progress, a banner above the file list names it and shows the number of conflicted files; "Résoudre" opens the resolution screen. The Git operation itself is started elsewhere; this screen handles what it leaves behind.
- Each conflicted file shows its kind (modified on both sides, added on both, deleted on one side and modified on the other…), the common base, and both versions named after the operation: in a merge "votre branche main" against "la branche fusionnée feature"; in a rebase the sides are swapped by Git, so they are named "la base du rebase" and "votre commit rejoué <commit>".
- A file is resolved by keeping one side (for a deletion conflict, accepting the deletion or keeping the modified file), or by writing the result in the editable box, which starts from Git's merged text. A result that still contains conflict markers is refused unless you confirm. A result is never written through a symbolic link or outside the repository. Binary files only offer the choice between the two versions.
- Resolved files stay listed. If one is modified again after its resolution, it is shown as "à valider de nouveau" and continuing stays blocked until it is validated again.
- "Continuer" is offered only when no file is in conflict and none is stale; it runs `git commit` (merge) or `git <operation> --continue` with your profile identity, hooks included, and the screen reports new conflicts if the operation stops again. "Abandonner" asks first, copies the files you had worked on to `.git/gitpoulpe-backup/<time>-<operation>/`, then runs `git <operation> --abort`; untracked files are untouched.

## Merging

- On the branch screen, select a branch other than the current one and choose "Fusionner dans « <current> »…". A preview shows both branches by name, whether Git will fast-forward, create a merge commit or has nothing to do, how many commits each side brings, the files predicted to conflict (computed with `git merge-tree`, without touching your work) and the files with local changes.
- The merge only runs if the current branch is still where the preview saw it, if no other operation (merge, rebase, cherry-pick, revert) is in progress and if the source exists. "Toujours créer un commit de fusion" forces a merge commit even when a fast-forward is possible. The merge commit uses your profile identity (and signing), hooks run as configured, and the message is Git's usual `Merge branch '<name>'`.
- Local changes are never overwritten: Git refuses a merge that would, and GitPoulpe reports the files and changes nothing. "Mettre mon travail local de côté" runs the merge with `--autostash`; if putting the work back conflicts, the work stays in `git stash list`, which the result message says.
- A merge that conflicts opens the resolution screen above; the merge can then be continued or abandoned from there. Merges are recorded in the per-repository journal.

## Stash

- "Réserve" on the branch screen lists the stashes (message, branch, date) and, for the selected one, its files: modified ones, whether they were staged, and the untracked files it holds.
- "Mettre de côté" creates a stash with a message and a scope you choose: tracked changes (default), plus untracked files, keeping the index in place, or only the staged changes (which excludes the other options). It refuses when there is nothing to stash for that scope, and during a merge, rebase, cherry-pick, revert or unresolved conflicts. The stash is created with your profile identity.
- A stash is always addressed by its identity, never by its position in the list, so a list that changed behind the screen cannot make an action hit another stash. "Appliquer" keeps the stash, "Appliquer et retirer" removes it only once the application succeeded, and "Restaurer l'index" re-stages what was staged.
- An application that conflicts keeps the stash and opens the resolution screen ("application du stash"), where the files can be resolved; nothing is aborted or deleted for you.
- Deleting a stash is explicit: it names the stash and shows the `git stash store -m <message> <id>` command that brings it back, which works as long as Git has not cleaned its unreachable objects.

## Publishing a branch

- "Publier…" on the branch screen opens a panel for the selected branch: the remote (a choice when there are several), the exact destination (`refs/heads/<branch>` on that remote, always given as an explicit refspec) and what will happen, from the last fetch: new branch, N commits to send, already up to date, behind, or diverged. Nothing is sent without that preview.
- A normal publication never overwrites anything: if the remote moved or the histories diverged, Git refuses, your work stays untouched and the message says to fetch and integrate, or, if you rewrote the history yourself, to use the protected publication. A refusal by a remote hook is reported as the remote's decision. The branch can be set to track the remote branch (on by default for a new branch).
- A rewritten history can only be published through `--force-with-lease` with the exact remote commit known at the time (the remote-tracking branch of the last fetch), after a confirmation that names that commit and lists the remote commits that would stop being reachable. If somebody pushed since, the push is refused and nothing is overwritten. There is no plain force.
- After a successful push the remote branch is read back (`ls-remote`) and compared with what was sent; a remote that did not keep the value (a server hook, for instance) is reported. The transfer shows its progress and can be cancelled; SSH, HTTPS and local remotes behave as described above, and a `pre-push` hook runs as configured. Pushes are recorded in the per-repository journal.

## Reflog

- "Reflog" on the branch screen lists the entries of the `HEAD` reflog or of the current branch's reflog, newest first, with the selector (`HEAD@{n}`), the operation and its message, the commit and the date. The file is read directly, so an entry whose object has been pruned is still listed and marked "objet absent".
- Selecting an entry inspects its commit (subject, author, parents) and says which branches or tags still reach it; a commit that only the reflog keeps is called out.
- "Créer une branche de récupération" gives a name to an old commit without moving or touching the current branch, the index or the working tree (an existing name is refused). An object that is no longer in the repository can be neither inspected nor recovered, and the screen says so instead of reporting a recovery. Recoveries are recorded in the per-repository journal.
- Reflogs are local and temporary: Git eventually expires them, which is why the recovery branch is the durable mark.

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
