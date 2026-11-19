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

## Resetting a branch

- On a commit of the history, "Repositionner la branche ici…" opens a preview for a `soft`, `mixed` or `hard` reset of the current branch to that commit. Each mode is described with its exact effect: `soft` only moves the branch; `mixed` also resets the index (staged files become unstaged) and keeps the files; `hard` resets the index and the tracked files. The commits that leave the branch are listed, as are the staged and unstaged files.
- A `hard` reset that would erase unsaved changes lists those files and needs a second, explicit confirmation; the git layer refuses it without that confirmation. It refuses as well when the branch moved since the preview, when the target is not a full commit id of this repository, and during a merge, rebase, cherry-pick, revert or unresolved conflicts.
- Before moving anything, the previous tip is pinned under `refs/gitpoulpe/recovery/<time>-<id>-head` (it survives the expiry of the reflog and garbage collection), and the tracked unsaved changes of a destructive `hard` reset are snapshotted with `git stash create` under `…-work`. Limits that the screen states: untracked files are not touched and ignored files are not saved. Resets are recorded in the per-repository journal.

## Cherry-pick and revert

- On a commit of the history, "Ajouter au cherry-pick" and "Annuler ce commit (revert)" build a queue: the commits are listed in the order they will be applied, and can be moved or removed before running. The preview shows the target branch, the number of commits and the files with local changes.
- Cherry-pick applies the commits in the announced order. Choose what happens to a commit that has become empty: ignore it (default), keep it empty, or stop. Revert creates the inverse commits and leaves the history in place.
- A merge commit has several parents, so its parent is chosen explicitly (nothing is preselected) and merge commits are only accepted together, never mixed with ordinary commits.
- A conflict in the middle of the sequence stops it and opens the resolution screen above, which names the sides after the operation; the remaining commits are kept by Git, "Continuer" resumes them, "Passer ce commit" drops the current one and "Abandonner" restores the branch as it was before the sequence. Local changes that would be overwritten stop the run untouched. The commits use your profile identity and signing; an operation already in progress or a branch that moved since the preview blocks the run.

## Undo and redo

- "Annuler/refaire" on the branch screen lists the operations GitPoulpe performed on this repository (kept in its preferences, outside the repository). Only the ones whose inverse is defined offer "Annuler": a commit, an amend, a merge, a cherry-pick, a revert or a reset (the branch goes back to where it was), the creation, deletion or renaming of a branch, and the creation or deletion of a tag. A push, a stash operation or a change to the remotes are listed as not reversible, with the reason.
- The state is read again before every undo or redo: the branch or reference must still be exactly where the operation left it (or, for redo, where it started), the name must be free, and no merge, rebase or conflict may be in progress. If another tool changed anything, nothing is touched and the message says what was expected and what was found.
- Moving the current branch back uses `git reset --keep`, so unrelated local changes survive and an undo that would overwrite local work is refused; a branch that is not checked out is moved with a compare-and-swap `update-ref`; a branch checked out in another worktree is left alone. The undo of a hard reset cannot bring back unsaved changes (they are in `refs/gitpoulpe/recovery`), and the screen says so.
- An operation is recorded only once it succeeded, so an interruption between the operation and its record leaves the repository consistent: the operation is simply not offered for undo (the reflog still has it). Operations finished after resolving conflicts are not recorded.

## Interactive rebase

- On a commit of the history, "Rebase interactif depuis ce commit…" takes that commit as the base and lists the commits after it, oldest first, each as `pick`. Touch an action to cycle through `pick`, `reword` (with the new message), `edit`, `squash`, `fixup` and `drop`, and move commits up to reorder them. Ranges that contain a merge commit are refused.
- The plan is validated before anything is rewritten, and the problem is shown live: every commit must appear exactly once (a commit is removed with an explicit `drop`), the first kept commit cannot be `squash` or `fixup`, a `reword` needs a message, and at least one commit must stay. The rebase also refuses if the branch moved since the plan, if another operation is in progress, or if local changes would be in the way (an option puts them aside with `--autostash`).
- Before rewriting, the old tip is pinned under `refs/gitpoulpe/recovery/<time>-<id>-rebase` (Git also sets `ORIG_HEAD`). A conflict or an `edit` step pauses the rebase and opens the resolution screen, which names the sides after the rebase (the base, and your replayed commit); from there it is continued, the commit is skipped, or it is abandoned, which restores the old tip. A finished rebase is recorded in the undo log.

## Integrating remote changes

- For the current branch, "Intégrer les changements distants…" shows the tracked branch (for example `origin/main`), how many commits each side has since the last fetch, and the strategy, which you choose explicitly: fast-forward only, merge, or rebase. Nothing is chosen for you, and your `pull.rebase` setting is ignored.
- The run fetches first, then integrates with the chosen strategy on what was fetched. If the fetch fails or is cancelled, nothing is integrated and the working tree, the index and the local branches are exactly as they were. A fast-forward-only pull on diverged branches stops with an explanation (the remote references are still updated); a merge produces `Merge remote-tracking branch '<upstream>'`; a rebase replays your commits and pins the old tip under `refs/gitpoulpe/recovery`.
- A conflict is never hidden: it opens the resolution screen of the merge or the rebase, from which it is continued or abandoned. Local changes that would be overwritten stop the integration untouched, or are put aside with the `--autostash` option (a reapplication that conflicts is surfaced too and the stash is kept). A branch without a tracked upstream, a detached `HEAD`, another operation in progress, or a branch that moved since the preview are refused first. A successful integration is recorded in the undo log.

## Worktrees

- "Worktrees" on the branch screen lists the repository's worktrees: path, branch, and each one's own state (staged, modified and untracked counts, locked, or "dossier disparu" when the directory vanished). "Ouvrir ce worktree" switches GitPoulpe to it; the other worktrees' files are never touched.
- A new worktree is created at an absolute path that does not exist yet (not inside the open worktree), on a new branch or on an existing one. A branch that is already checked out in another worktree is refused with that worktree's path, as are an invalid branch name, a missing parent folder or a relative path.
- Locking (with an optional reason) protects a worktree from removal until it is unlocked. Removing one is controlled: the main and the open worktrees cannot be removed, a locked one needs unlocking first, and a modified one lists its modified and untracked files and needs a second explicit decision. The tracked changes are then saved under `refs/gitpoulpe/recovery/…-worktree-…`; untracked files cannot be saved and the screen says how many are lost. The branch is always kept. "Nettoyer les orphelins" prunes entries whose directory is gone.

## Hooks

- "Hooks" on the branch screen shows the effective hooks directory (resolved by Git, so a custom `core.hooksPath` is honoured, with the value and the scope it comes from: local, global or system), whether that directory belongs to the project or sits outside the repository, and every file in it: active hooks, disabled ones (a known hook that is not executable, which Git ignores) and files that are not Git hooks. `*.sample` files are left out.
- Activation is the standard Git mechanism, the execute bit: "Désactiver" / "Activer" changes it on that hook only. A directory outside the repository may serve several repositories, so changing a hook there first asks for an explicit confirmation of that scope; a hook that is a symbolic link is never changed from here (its target could be shared), and only known hook names are accepted.
- Hooks run as configured on every operation and are never bypassed (no `--no-verify`). When a hook fails, its cause stays visible and its output is filtered: password, token, secret, API key and authorization values, well-known token formats (GitHub, GitLab, Slack, AWS, `sk-`) and private-key blocks are masked, on top of the credentials already masked in URLs. The same filter applies to every Git diagnostic GitPoulpe shows.

## Git LFS

- "LFS" on the branch screen detects whether Git LFS is installed (`git lfs version`) and lists the files that the attributes send to LFS. Reading them needs no tool: the pointer committed in the repository is parsed strictly (version line, a 64-hex SHA-256 `oid`, a `size`), and each file is classified: real content present (its size and SHA-256 match the pointer), pointer in the working tree with the object available locally, object absent (to be fetched), content modified (it no longer matches the versioned object), or committed as real content despite the LFS attribute.
- Without Git LFS on the machine, the screen says so, lists the pointers and explains how to enable the support: install the `git-lfs` package with your system's package manager and run `git lfs install` once. GitPoulpe never installs or downloads anything by itself, and no transfer is attempted.
- With the tool, "Récupérer", "Télécharger" and "Envoyer" run `git lfs fetch`, `pull` and `push` through the same network layer as the other remote operations (your SSH and credential setup, repository-supplied helpers ignored), with progress, a cancel button that stops the tool and its children, and distinct messages for access refused, repository or object not found, server error and network failure, with secrets masked in the output.
- Limit: the transfers are tested against a stand-in `git-lfs` program (the real tool is not available on the development machine), so the exact progress lines of a real server remain to be confirmed.

## Submodules

The **Submodules** button on the branches screen lists every submodule, nested ones included, with the commit the repository expects, the commit actually checked out, how far ahead or behind it is, and any local changes.

- **Initialise** registers a submodule; **Update** puts it on the expected commit and shows progress, with a cancel button.
- A submodule with uncommitted changes is never updated. If it holds commits that no branch contains, the update asks for confirmation first and keeps them under `refs/gitpoulpe/recovery/` inside the submodule.
- URLs are shown with credentials masked. URLs using `ext::` or `fd::` transports, or starting with a dash, are refused and never handed to Git.
- **Open** loads the submodule as a repository of its own.

## Gitflow

The **Gitflow** button on the branches screen manages feature, release and hotfix branches on top of the regular branch, merge and tag operations.

- The effective convention is always shown: main and develop branch names, the feature, release and hotfix prefixes and the version-tag prefix (`gitflow.*` keys, the same ones git-flow-avh uses). Until you save it, the defaults are only displayed and nothing is written to the repository. Saving lists what will be written and, if the develop branch is missing, announces that it will be created from the main branch.
- Starting and finishing list every step before anything runs. Starting a feature or release branches from develop, a hotfix from main. Finishing merges with a merge commit (never a fast-forward), tags releases and hotfixes with an annotated tag, merges into develop and deletes the finished branch.
- The announced steps are checked again right before running; if the repository changed in between, nothing runs. Tracked local changes, an operation already in progress and a tag that already points elsewhere (it is never moved) stop the flow before any step.
- A merge conflict stops the flow and opens the conflict resolution screen. After resolving it, finishing the branch again skips the steps already done and resumes with the rest.
- Flow steps are not recorded in the undo history; the reflog and the recovery markers remain available.

## GitHub accounts

The **Comptes fournisseurs** button on the profiles screen connects GitHub.com or a GitHub Enterprise Server with a personal access token. No GitPoulpe account exists or is needed.

- The token is validated against the server before anything is saved, then stored in the system keyring (Secret Service on Linux). Preferences only hold the account name, server and granted scopes, never the token. The field is masked.
- The account in use and the scopes the token grants are shown; fine-grained tokens, which carry no scope list, are labelled as such.
- Each connection is re-checked when the screen opens or on demand. An expired or revoked token, an organisation that requires SSO authorisation (with the authorisation link), a rate limit and a refused permission are reported as distinct, recoverable states: pasting a new token reconnects the same account.
- Servers must use HTTPS (plain HTTP is accepted for loopback only), redirects are never followed, and requests to another host are refused, so the token cannot leave the configured server.
- Disconnecting removes only that connection's token. Local Git profiles are not touched.

## File history

Each file header in the diff view has an "Historique" button that opens the history of that path. The list follows renames (`git log --follow`), so a file that was renamed several times shows each step with its path at that revision, and the row says "renommé depuis ..." where the path changed. A file that was deleted and recreated shows the deletion as its own step. Selecting a revision opens the exact diff of that commit for the file (with both paths, so a rename is read as a rename) and, with "Contenu", the file as it was at that commit. A deleted file, binary content and files over 1 MiB each get an explicit message instead of an empty pane, and a very large diff says it was cut. The list is limited to the 500 most recent revisions and says so. In a shallow clone, when the oldest revision shown is the shallow boundary, the screen states that the history is incomplete and how to complete it, instead of presenting the boundary as the file's origin. Refreshing keeps the selected revision; if it is no longer in the history, the screen says so and clears it.

The same screen offers the blame of the file. "Blame" attributes every line of the selected revision to the commit that wrote it (author, short id, line number), and follows the selection when you pick another revision; lines from before a rename keep their older path. "Blame du fichier local" attributes the working copy instead, and a line that is not committed yet is shown as "non enregistré" with its own colour instead of being attributed to a commit; staged lines are not committed either. Clicking an attributed line opens that commit in the history list; if the commit is not in the loaded list (the 500 most recent revisions), the screen says so instead of guessing. Attribution runs in the background and the lines are virtualized, so a file of thousands of lines stays readable; a file over 2 MiB is refused with an explicit message.

## Editing a file

In the unstaged view, each file header has an "Éditer" button that opens the working-tree file in a text editor (`Ctrl+S` or "Enregistrer" saves, `Échap` closes). Saving rewrites only that file, through a temporary file in the same folder that is renamed over it, so a failed save never leaves a half-written file or a stray temporary file, and the file keeps its permissions (an executable stays executable). Before overwriting, the file is compared with what was opened: if it changed on disk in the meantime, nothing is written and you choose explicitly between "Écraser avec mon texte", "Recharger depuis le disque (abandonner mon texte)" and "Garder mon texte". Line endings (LF or CRLF) and a UTF-8 byte order mark are kept as they were. Refused instead of rewritten silently: binary files, encodings other than UTF-8 (UTF-16, Latin-1...), files with mixed line endings, files over 512 KiB, symbolic links, folders, paths outside the working tree or inside `.git`, and read-only files or folders (the text you typed stays in the editor). Only working-tree files can be edited: there is no edit button in the staged view or on historical content, so an old revision is never written back by accident. Closing with unsaved changes asks first.

## External tools

"Outils…" (in the editor, file history and conflicts screens) configures an external editor, a comparison tool and a merge tool: an executable and its arguments, saved locally. The executable is started directly, never through a shell, and each argument is passed as it is written, so spaces, quotes, `$`, `;` and accented characters in paths reach the tool intact; quotes group words when you write the arguments. Arguments can use variables, checked when you save (an unknown one is refused): `{file}` and `{repo}` for the editor; `{left}`, `{right}`, `{file}` and `{repo}` for comparison; `{base}`, `{ours}`, `{theirs}`, `{merged}` and `{repo}` for merge. "Éditeur externe" opens the working-tree file (and refuses to start while the built-in editor holds unsaved text); "Comparer (outil externe)" in the file history compares the selected revision with its parent, or the working copy with the last commit when none is selected; "Outil de fusion externe" in the conflicts screen gives the tool the base, ours and theirs versions and the working file as the merged result. Versions are written to a private temporary folder that is removed when the session ends. A missing executable, a refused execution or a non-zero exit code is reported with its cause and nothing is lost: you can fix the configuration and run again. When the tool exits, the repository is read again, the built-in editor reloads the file if it holds no unsaved text, and the conflict list is refreshed; the scroll position is kept. Only one session runs at a time. No tool is preconfigured, and a tool that returns a non-zero code for "differences found" is reported as failed.

## Terminal

In the worktrees screen, "Terminal ici" opens an interactive terminal in the selected worktree (it asks you to select one first). The header names the shell and the exact directory, and the shell starts there, even when the path contains spaces or quotes. The shell is the one configured under "Outils…" ("Terminal (shell)": an executable and arguments, `{repo}` available), or else `$SHELL`, or `/bin/sh`. The session is a real pseudo-terminal with terminal emulation: colours (16, 256 and true colour), bold, inverse, cursor, full-screen programs and arrow, function and Ctrl keys are supported, and the screen follows the window size (the program is told the new size). Keys go to the program, including Escape and Ctrl+C; the "Interrompre (Ctrl+C)" button stops the running command and "Copier l'écran" (or Ctrl+Shift+C) copies the visible text. Output is coalesced so a command that prints a lot never freezes the interface. When the shell exits the screen says so with its exit code. Closing the terminal, leaving for another screen, switching repository or quitting kills every process of the terminal's session, background jobs included, so no command is left running; Git is then read again so changes made from the terminal appear. Linux only for now; pasting is not supported yet.

## Command palette

`Ctrl+K` opens the command palette above the current screen (and closes it again); the screen below keeps its place and stays up to date. Type to search by words, ignoring case and accents, in the command's name and its keywords: every word you type must match. `↓` and `↑` move through the results (and wrap), `Entrée` runs the highlighted command, `Échap` closes the palette and gives the focus back to the control that had it, and a click runs a result. Only the commands that can actually run in the current situation are listed, recomputed live: without a repository only opening, cloning, accounts, profiles, tools and theme; with a repository the screens (history, branches, tags, remotes, stashes, reflog, worktrees, hooks, LFS, submodules, Gitflow, pull requests, ignored files, undo), refresh and the unified/split switch; "Résoudre les conflits" only during a merge, rebase or similar operation; and the file commands (history, edit, stage, unstage) only when a file is current in a view where they make sense. A command runs against the repository and the file that are current when you press `Entrée`, never the ones from when the palette was opened, and a command that stopped being available in the meantime is not run. The palette is not available while the terminal has the keyboard.

## Keyboard shortcuts

The shortcuts of the file views and the application can be changed: "Raccourcis clavier…" in the command palette (`Ctrl+K`) opens the list of every action with its effective key, searchable by action or by key, and customised entries are marked. Select an action and press "Modifier", then press the new key (`Échap` cancels); "Rétablir" restores the default of one action and "Tout rétablir" restores them all. Choices are saved locally and the keys shown in the footer follow them. Before anything is applied the new key is checked: a key already used by another action is refused and the action that owns it is named; the copy, paste, cut, select-all, undo and redo combinations are reserved; reading actions (next file, view, stage and so on) accept only a single key without Ctrl so they can never hijack typing, and global actions (open, refresh, palette) need Ctrl plus a key or a function key; keys such as Enter, Tab or the arrows cannot be assigned. A key pressed while choosing a new combination is never executed. Restoring a default that another action has taken in the meantime is refused with the reason instead of creating a duplicate. Reading shortcuts do nothing while a text field has the focus or another screen is open, and everything works the same in the light and dark themes. Customised keys only apply to the actions in the list; the arrows, `PgUp`, `PgDn`, `Home`, `End` and `Échap` keep their fixed meaning.

## Workspaces

"Espaces de travail…" (command palette, `Ctrl+K`) groups local repositories into named workspaces, stored on this machine only, with no account and no network access. You create a workspace, rename it (names are unique, ignoring case), add the current repository or any folder (only real Git repositories are accepted, once each), remove a repository from it, or delete the workspace after a confirmation; removing or deleting only edits the list and never touches a file of any repository. Each repository remembers the view you were reading (staged or not) and the file you were on, saved when you leave it for another repository of the workspace and when the screen opens. "Restaurer l'espace" reopens the last repository of the workspace with that view and file, offline. A repository that was moved or is no longer a Git repository is flagged "introuvable" or "n'est plus un dépôt Git" with a "Réparer…" button to point it at its new folder; the rest of the workspace keeps working, restoring skips the broken repositories and says how many were skipped, and opening a broken one explains how to repair it. Paths that are not valid UTF-8 cannot be saved.

## Favorites

"Favoris…" (command palette, `Ctrl+K`) keeps a local list of favorite repositories, stored on this machine. Local favorites are references to a folder: "Ajouter le dépôt courant aux favoris" in the palette, or the button in the screen. Remote favorites come from the account browser: "Ajouter aux favoris" on the selected repository remembers the account that gave access, the full name and the clone address. The list is searchable (case and accents ignored, name or path or address) and each entry says what it is: "référence locale" (with its path and whether it is still there) or "accès fournisseur" (with the account and whether its access was checked). Opening a local favorite opens the repository; opening a remote one prefills the clone screen. A moved folder shows "introuvable", opening it explains why and "Réparer…" lets you point it at the new place (a folder that is not a Git repository is refused); a favorite whose account was disconnected shows "compte déconnecté", and "Vérifier l'accès" checks the account against the provider, so a revoked token shows "accès refusé" with the provider's reason; in both cases opening refuses and points to "Comptes fournisseurs" to reconnect. Removing a favorite only edits the list: the repository and the connection are never touched. A repository can be a favorite once, and the same remote name on two different accounts counts as two favorites.

## Multi-repository overview

"Vue multi-dépôts…" (command palette, `Ctrl+K`) shows the state of several repositories at once: the open repository, the repositories of every workspace and the local favorites, each listed once under its own path with the sources it comes from. Pick the ones to follow with their `[x]` box ("Tout sélectionner" and "Aucun" too). For each selected repository the line shows its branch (or detached HEAD), the distance from its upstream (↑ ahead, ↓ behind, or "pas de suivi distant"), the number of staged, modified and untracked files, conflicts, and a merge, rebase, cherry-pick, revert or bisect in progress, plus the position saved in a workspace (view and file). Every figure belongs to the repository named on its line, and a failure stays on its line: a folder that is no longer a Git repository, a missing folder or a repository that is too slow each show their own message while the others keep their results. The reads are bounded: at most four repositories are read at the same time, each read gives up after 30 seconds ("délai dépassé"), the status is read without taking any lock or running the repository's file-system monitor, and changing the selection or refreshing cancels the reads that are no longer wanted, so a late answer for an old request is ignored. "Ouvrir" joins the repository's own session; if it belongs to a workspace (the active one first) it comes back with the saved view and file, and a missing repository explains how to repair it instead of opening nothing. Reads only; nothing is fetched from the network.

## Notifications

"Notifications…" (command palette, `Ctrl+K`) is the application's inbox; the footer counts the unread ones. Local events are recorded as they happen (a merge, rebase, cherry-pick or revert that stopped and needs its conflicts resolved). "Actualiser" reads the provider events of every connected account that is not muted: GitHub notifications (review requested, mentions, CI activity, other activity) and GitLab to-dos (review requested or approval required, mentions, failed pipelines, other), unread ones only, each with its repository and pull or merge request number. Identical events are ignored, and a burst on the same subject (same account, repository, kind and number) is grouped into one line with its count ("×6") that becomes unread again; the same pull request seen through two accounts, or with two kinds of event, stays separate. Preferences are saved locally: notifications on or off, each kind on or off, each account on or off; a muted kind or account is neither recorded nor shown, a muted account is not even asked, and nothing is read when notifications are off. "Ouvrir" marks the notice read and goes to the right place: a conflict notice opens its repository and the conflicts screen; a provider notice finds, among the open repository, the workspaces and the favorites, the repository whose remote matches that account's server and repository name, opens it and selects the pull or merge request in the list (if it no longer exists or is no longer visible with that account, the list says so and shows nothing else); if the repository is not cloned anywhere the clone screen is prefilled with its address. Every notice belongs to its own account: if that account was disconnected, or the provider refused its access at the last read ("Lecture impossible pour ..."), opening is refused with the reason and a pointer to "Comptes fournisseurs", and a late answer for a removed account is dropped, so nothing from another connection is ever shown in its place. The inbox keeps the 200 most recent notices. Notifications are in-application only; there are no system pop-ups yet.

## Issues

"Issues du dépôt courant…" (command palette, `Ctrl+K`) reads the issues of the repository you have open, on GitHub and on GitLab, with the account whose server and repository name match one of its remotes (with several matching accounts, "Changer de compte" switches between them and the list is replaced, never merged). The list can be filtered (open, closed, all), searched, and followed over every page; GitHub's pull requests, which its issues API mixes in, are left out, and a list that hit the provider's cap says it was truncated. Selecting an issue shows its author, labels, description and comments, and its links: the pull or merge requests that reference or close it (each listed once, with a button that opens that request in the pull request list, or tells you it no longer exists or is not visible with that account), the commits that mention it ("Voir le commit" opens it, or says the commit is not in the local repository and that remote references must be fetched), and the local branches whose name contains the issue number as a whole number (`feature/105-fix` for #105, not `feature/1050-x`). Rights are the account's: a repository it cannot see, a revoked token, or a repository with issues disabled each give an explicit message and clear what was shown before instead of leaving stale content. Differences between servers are displayed rather than hidden: for GitLab the server version is shown, and a self-managed instance that does not provide the linked merge requests is reported as "Limite : ...", as is a GitHub account that cannot read the issue's timeline. Read-only: nothing is created, edited or closed.

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

Signing in from the browser uses the GitHub device flow: the app shows a short code and an address, you authorize it in your browser, and the token is stored in the system keyring. It needs no client secret. It is available for github.com only; GitHub Enterprise Server and machines without a browser use a personal access token.

A GitHub Enterprise Server is reached only at the address you typed. An untrusted certificate is reported as such and refused before any request is sent, an unreachable server is reported as unreachable, and neither case falls back to github.com or stores the token.

GitLab accounts connect the same way with a personal access token, on gitlab.com or on a self-managed instance (HTTPS required, plain HTTP only on loopback). The screen shows the account, the scopes the token really has and its expiry date, and an expired, revoked or blocked token, a rate limit and a token that belongs to another account each get their own message. Several accounts on several servers coexist, each with its own keyring entry, and disconnecting one removes only its token. Browser sign-in is offered for github.com only; for older GitLab servers that do not describe the token, the account connects with no scope information.

### Browsing organizations and repositories

From the accounts screen, "Parcourir les dépôts" lists the repositories of the selected account or of one of its organizations, with a filter on name and description. Each row shows whether the repository is private, a fork, archived or read-only, and "Cloner ce dépôt" opens the clone screen with its URL filled in. The header always names the account and server in use, and switching account discards any answer still in flight for the previous one. Two accounts or servers with same-named repositories never mix, and a next-page link to another server is refused. SSO, rate-limit, expired-token and permission failures stay distinct, and a failure never falls back to another server. The list is capped; when it is truncated the screen says so. Cloning a private repository still relies on your Git credentials.

For GitLab accounts the same screen lists groups and subgroups with their full path ("acme/platform/infra"), the projects of a group including its subgroups, and the projects you can reach directly, including those shared with you without group membership. Each project keeps its full path, whether it is private, archived or a fork of another project, and whether your access level allows pushing. Forking is offered for GitHub accounts only for now.

The detail panel can also fork the selected repository into your account or one of your organizations. GitPoulpe first looks for an existing fork at the destination, so it never creates a duplicate; a destination name taken by an unrelated repository, a refused destination and a fork still being created by GitHub each get their own message, and none of them touches your local repository or its remotes. The result names the fork and its source, and "Cloner le fork" opens the clone screen with its URL.

Signing in from the browser uses the GitHub device flow: the app shows a short code and an address, you authorize it in your browser, and the token is stored in the system keyring. It needs no client secret. It is available for github.com only; GitHub Enterprise Server and machines without a browser use a personal access token.

### Browsing organizations and repositories

For a connected GitHub account the hosting crate lists the organizations the token can see and the repositories of the account or of one organization. Pages are followed up to a fixed cap and the result says when it was truncated. Each repository carries the id of the connection it came from, so two accounts or servers with same-named repositories never mix. A next-page link to another server is refused. SSO, rate-limit, expired-token and permission failures stay distinct. There is no screen for this yet.

### Pull requests

From the branches screen, "Pull request" prepares a GitHub pull request for the current branch. The repository's remotes are matched against your connected accounts (`origin` first), and the screen names the account and repository in use; with several matching accounts you can switch between them. It shows the source and target branches and the changed files, with commit counts, before anything is sent. From a fork you can target the original repository instead, in which case the head is sent as `owner:branch`. You choose the title, description, target branch and whether it is a draft. A branch that is not published, a pull request already open for the same branches, a missing title and a refusal from GitHub each stop with their own message and create nothing; after a successful creation the button disappears, so a pull request is never created twice.

"Voir les pull requests" lists the pull requests of the target repository, filtered by open, closed or all, with their real state (open, draft, closed, merged). Selecting one shows its author, its head and base branches with the exact revisions, its description, its commits and every changed file with its diff, read in one continuous scroll. Each selection is loaded independently, so switching pull requests never shows a late answer for the previous one. If the head moved while you were reading, or the pull request changed while it was loading, a notice says so and "Actualiser" rereads it. Diff text comes from the GitHub API (at most 3000 lines are displayed); very large pull requests are handled by a later, Git-based reader.

In the same view you can comment. Clicking a line of a diff anchors a comment to it; the comment goes to the exact revision shown, on the old or new side you clicked, and a range can be selected on one side with "Sélectionner une plage de lignes". Existing threads are shown under their file with their replies, marked "périmé" when their line no longer exists, and "Répondre" adds to a thread. General comments are supported too. If the pull request moved since you loaded it, a line comment is refused with an explicit message instead of landing on the wrong line. If a send fails with an unknown outcome (network or server error), your text is kept and "Vérifier si le commentaire est parti" reloads the thread and tells you whether it was already published, so a retry never duplicates it.

Reviews are prepared as a draft: "Ajouter à la revue" collects line comments across several files, together with a review message, and the draft stays attached to its own pull request when you switch to another one and come back. One review is then sent in a single request, with the decision you picked: "Commenter", "Approuver" or "Demander des changements". The screen names the account the review is posted as, and published reviews are listed with their decision and revision. If the pull request is updated while you review, the draft is kept, the comments on files that changed are flagged "À RELIRE", and nothing is sent until you adopt the new revision and mark them read or remove them. Approving or requesting changes on your own pull request, requesting changes without a message, and a revision that moved before sending are all refused with an explicit message and never shown as a success. If the outcome of a send is unknown, "Vérifier si la revue est partie" checks the pull request before you retry. Drafts are kept for the session only; they are not saved to disk.

The pull request view also shows the CI checks of the revision it currently displays: checks and legacy commit statuses are listed with their state (pending, succeeded, failed, cancelled, skipped) and a link to the provider's details, with a count per state. A revision with no check says so instead of looking green, and an unavailable API is reported as an error, never as an empty or successful list. When a new revision appears, the results of the previous one are dropped at once, with a notice, and an answer that belongs to an older revision or another pull request is ignored.

For an open pull request, the "Fusion" section reads the repository's real rules before offering anything: only the merge methods the repository allows are proposed (merge commit, squash, rebase), each with its effect on the history, and the blockers are listed explicitly (draft, conflicts, branch behind, required checks that are failing, pending or missing, missing approvals, requested changes). Merging takes two steps and names the exact revision you read; if the head moved since, nothing is sent. A refusal from GitHub is shown as a refusal. After a merge the remote state is read again before success is claimed, and if the outcome of the request is unknown, "Vérifier l'état distant" tells you whether it was merged before you retry. If the branch protection rules cannot be read with your account, the screen says so and GitHub enforces them at merge time.

The GitHub API caps a pull request's file list at 3000 files, so "Lire toute la PR depuis Git" reads the exact revisions instead: it fetches the pull request's head (`refs/pull/N/head`) and, if needed, the target branch from the remote that matched your account, computes the merge base with Git, and opens the commit reader on that range. All files are listed without truncation and their diffs are read progressively by the same virtualized reader used for commits. If a revision cannot be fetched (the pull request moved or was deleted, no common ancestor, network or disk error), the screen says so and does not claim the pull request was loaded. Cloning access for private repositories relies on your Git credentials. The rendering benchmark on reference GPU hardware is a separate measurement that has not been run here.

### GitLab merge requests

With a GitLab account the same screens create and read merge requests. A project in nested groups ("acme/platform/api") is recognised from its remote, and the branch is compared with its target before anything is sent, with the commit counts and the changed files counted from the diffs. From a fork the merge request is created in the fork with the parent project as its target. A draft is sent by prefixing the title with "Draft:", once. A branch that is not published, a merge request already open for the same source project, source branch and target branch, a missing title and a refusal from GitLab each stop with their own message and create nothing.

The list shows opened, closed (including merged) or all merge requests with their real state. A selected merge request shows its revisions, description, commits and every page of its diffs in one continuous scroll. Files GitLab returns without a diff (binary, too large) say so. If the server has no paginated diffs endpoint, the older single-response one is used and a possible overflow is shown as a truncated file list; "Lire toute la PR depuis Git" then reads the revisions from `refs/merge-requests/N/head` instead. A merge request that moved while it was loading is flagged. Reviews and merging work on GitLab too, with these differences. Discussions are read with their replies; a thread whose position belongs to an older revision is marked outdated. A line comment is sent to the exact revision shown, with the old and new line numbers GitLab requires (read from the diff, so context lines and removed lines both work); only lines present in the diff can be commented, and a range of lines is refused. A review is prepared as GitLab drafts and published in one step; with "Approuver" the approval, tied to the revision, is recorded before publishing, and if anything fails the drafts are withdrawn so nothing half-sent remains. GitLab has no review message of its own, so a review with line comments needs a message, which is also how a lost send is verified. "Demander des changements" is not offered because the API does not allow it. Pipelines show the jobs of the merge request's pipeline for the revision displayed (a fork's pipeline is read in the fork); jobs allowed to fail are labelled. The merge section reads the project's merge method and squash setting, the required approvals, whether the pipeline is required, the protected target branch against your access level, and GitLab's own merge status, and lists every blocker in words. Merging names the revision, is confirmed in two steps, and is read back before success is claimed. It never rebases or schedules a merge on your behalf.

## Importing a patch

"Importer un patch…" (command palette) opens a patch file made by `git diff` or `git format-patch`. The patch is inspected first: files touched, added and removed lines, binary files. Nothing changes until you choose where to apply it (working tree, index, or both) and run "Vérifier", which must report that the whole patch applies. Application is all-or-nothing. An optional three-way fallback applies what it can and leaves conflict markers in the files. Patches over 8 MiB and paths escaping the repository (absolute, `..`, drive letters, backslashes) are refused.

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
