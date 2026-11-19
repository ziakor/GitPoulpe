# GitPoulpe user guide

This guide covers what the current build does and what it does not. Each feature has a longer reference section in the [README](../README.md). The interface itself is in French; button and command names below are quoted as they appear on screen.

Check your build with `gitpoulpe --version`. This guide describes the source tree it ships with; if the version or commit differs from the one you run, trust the binary.

## What is supported

| | Status |
| -- | -- |
| Linux (graphical session, GPU) | Supported. This is the only platform it has been built and tested on. |
| Windows, macOS, WSL | Not available yet. |
| GitHub.com pull requests, GitLab merge requests | Available (tested against simulated servers, not yet against real accounts). |
| GitHub Enterprise Server | Accounts can be connected; not verified against a real instance. |
| Bitbucket, Azure DevOps, Jira, Trello | Not available. |
| AI features, agents, automations | Not available. |
| In-app update | Not available: install a new archive over the old one. |

Anything not listed in this guide or the README is not implemented.

## Install

From the Linux archive (`dist/gitpoulpe-<version>-linux-<arch>.tar.gz`, built with `packaging/linux/build.sh`):

```sh
tar -xzf gitpoulpe-*.tar.gz && cd gitpoulpe-*/
./install.sh            # default prefix ~/.local, or --prefix /absolute/dir
gitpoulpe --version
```

The script refuses to install if shared libraries are missing and tells you which; Git must be installed. To remove it: `./uninstall.sh` (keeps your preferences; `--purge-config` deletes them; repositories are never touched). From source, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Open a repository

- Run `gitpoulpe /path/to/repository`, or start it and use "Ouvrir un dépôt…" (`Ctrl+O`), or "Cloner un dépôt".
- Reading a repository never changes it. The unstaged and staged views show every file with its diff in one scrolling stream.
- `Ctrl+K` opens the command palette: type a few words to find any screen or action.

## Everyday work

1. **Stage**: "Indexer" on a file, a block or selected lines (see [Writing](../README.md#writing)).
2. **Commit**: write the message and commit with your Git profile ("Profils Git" in the palette sets name, email and signing). Hooks run as configured.
3. **Branch**: click the branch name in the top bar to create, switch, rename or delete branches.
4. **Share**: "Intégrer les changements distants…" brings in remote work with the strategy you choose; "Publier…" sends a branch after showing exactly what will happen.
5. **Review**: "Pull requests et merge requests" in the palette lists them, shows the diff against the exact commits, and lets you comment, review and merge once an account is connected ("Comptes fournisseurs").

Other tools, each documented in the README: history and file history, blame, stash, tags, remotes, worktrees, hooks, Git LFS, submodules, Gitflow, interactive rebase, cherry-pick and revert, patches ("Importer un patch…", "Exporter en patch…"), links ("Ouvrir un lien…"), workspaces, favorites, multi-repository overview, notifications, issues, external tools and an embedded terminal.

## Risky operations and how to get back

| Operation | What protects you | How to recover | Limits |
| -- | -- | -- | -- |
| Delete a branch | A branch that is not merged lists the commits only it holds and needs an explicit second step. | The tip is in the per-repository journal; recreate the branch from it. | Remote branches and tags are not touched by that screen. |
| Reset (soft, mixed, hard) | Preview of what leaves the branch; a hard reset that erases changes needs a second confirmation. | Old tip pinned under `refs/gitpoulpe/recovery/…-head`; tracked unsaved changes under `…-work`. "Annuler/refaire" can move the branch back. | Untracked and ignored files are not saved. |
| Interactive rebase | Plan validated before rewriting; refuses if the branch moved. | Old tip pinned under `refs/gitpoulpe/recovery/…-rebase`; abandoning a paused rebase restores it. | Ranges that contain merge commits are refused. |
| Merge, pull | Preview first; local changes are never overwritten. | Abandon from the conflict screen; "Annuler/refaire" for a finished merge. | Operations finished after resolving conflicts are not recorded for undo. |
| Publish (push) | Explicit refspec and a preview; never overwrites by default. | A rewritten history goes only through a protected push that names the remote commit it will replace. | A push cannot be undone from the app. |
| Lost commit | The reflog lists old states. | "Reflog" then "Créer une branche de récupération". | Reflogs expire; pruned objects cannot be recovered. |
| Apply a patch | Inspected first; all-or-nothing; unsafe paths refused. | `git apply -R` with the same patch file, before editing the touched files again, or restore them with Git. | The three-way option can leave conflict markers in files. |
| Edit a file | Saved atomically; refuses if the file changed on disk meanwhile. | Your editor text stays on screen after a refusal. | Binary and non-UTF-8 files are not editable. |

## Frequent problems

- **No window, or a GPU error at launch**: GitPoulpe needs a GPU and a graphical session; nothing is written to your repository in that case.
- **"index.lock" reported**: another Git process is running or crashed. Close it, then remove the lock yourself; the app never removes it.
- **"The branch moved since the preview"**: someone or something changed the repository after you opened the preview. Reopen the preview and check it again.
- **Push refused**: fetch and integrate first. Use the protected publication only if you rewrote the history yourself.
- **No account matches this repository**: connect the account for that host in "Comptes fournisseurs"; links and pull request screens only use a connected account matching the remote.
- **Preferences warning at startup**: the settings file is unreadable, from a newer version, or failed to migrate. The message names the copy that was kept; see [Preferences format](../README.md#preferences-format).
- **A link or patch is refused**: the reason is shown; links with parameters, encoded characters or unknown objects, and patches with paths outside the repository, are rejected on purpose.

## Commands in the palette

The palette (`Ctrl+K`) offers these commands. Some appear only when a repository, a file or a given view is open.

- Ouvrir un dépôt…
- Cloner un dépôt
- Comptes fournisseurs
- Profils Git
- Outils externes…
- Raccourcis clavier…
- Importer un patch…
- Ouvrir un lien…
- Exporter en patch…
- Issues du dépôt courant…
- Notifications…
- Vue multi-dépôts…
- Favoris…
- Ajouter le dépôt courant aux favoris
- Espaces de travail…
- Changer de thème
- Actualiser le dépôt
- Historique des commits
- Branches
- Étiquettes (tags)
- Dépôts distants
- Remisages (stash)
- Reflog
- Worktrees et terminal
- Hooks Git
- Git LFS
- Sous-modules
- Gitflow
- Pull requests et merge requests
- Fichiers ignorés
- Annuler ou rétablir une opération
- Résoudre les conflits
- Basculer entre vue unifiée et séparée
- Historique du fichier courant
- Éditer le fichier courant
- Indexer le fichier courant
- Désindexer le fichier courant
