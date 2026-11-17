# GitPoulpe

Client Git natif, gratuit et open source (GPLv3), écrit en Rust avec l'interface [Rune](https://github.com/anomalyco/rune). Linux d'abord ; Windows et macOS suivront.

État : le jalon M1, « Inspection locale », est livré. L'application lit un dépôt local et affiche ses changements ; elle ne modifie jamais le dépôt (ni index, ni fichiers, ni références).

## Ce que fait M1

- Ouvre un dépôt par le sélecteur de dossier (bouton « Ouvrir un dépôt… » ou Ctrl+O) ou par un chemin en argument. Dépôt ordinaire, sans premier commit, worktree lié. Un dépôt bare est signalé comme n'ayant pas de répertoire de travail.
- Deux vues, « Non indexés » et « Indexés » ; un fichier modifié dans les deux apparaît dans chacune avec son propre diff.
- Tous les diffs défilent dans un seul flux, en mode unifié ou séparé, avec sélection et copie. Les fichiers binaires, les conflits et les fichiers illisibles sont annoncés à leur place.
- Filtre par nom ou chemin.
- Actualisation sur changement du dépôt, au retour sur la fenêtre, ou à la demande (bouton Actualiser, touche `r`). La position de lecture est conservée.

Git est lancé comme programme externe, sans shell, avec les filtres, programmes de surveillance et accès réseau du dépôt neutralisés.

## Raccourcis

| Touche | Action |
| -- | -- |
| `/` | filtrer |
| `j` / `k` | fichier suivant / précédent |
| `1` / `2` | vue Non indexés / Indexés |
| `m` | mode unifié / séparé |
| `r` | actualiser |
| flèches, PgUp, PgDn, Début, Fin | lire |
| Ctrl+O | ouvrir un dépôt |

## Compiler et lancer

Prérequis : Rust 1.100.0 (`rustup toolchain install 1.100.0`), Git dans le `PATH` (testé avec 2.53.0), un GPU et une session graphique Linux. Le sélecteur de dossier passe par le portail XDG.

```sh
cargo +1.100.0 build --release -p gitpoulpe-desktop
target/release/gitpoulpe /chemin/du/depot
```

Sans argument, l'application s'ouvre sur l'écran d'accueil.

## Tests

```sh
cargo +1.100.0 test --workspace
cargo +1.100.0 clippy --workspace --all-targets -- -D warnings
```

## Limites connues

- Testé uniquement sous Linux.
- Mesure de performance (CV-05) : tenue avec réserve. En fenêtre réelle, une pause de 90 à 100 ms environ toutes les 10 s n'est pas expliquée. Rune ne fournit pas d'horodatage de présentation, donc aucun résultat sans fenêtre ne prouve 60 images/s.
- Une modification de `.gitignore` en cours de session n'est prise en compte par la surveillance qu'à la réouverture du dépôt.

## Organisation

Le code est découpé en trois crates : `gitpoulpe-core` (types et contrats), `gitpoulpe-git` (lecture de l'état et des diffs, cache, surveillance), `gitpoulpe-desktop` (interface).

## Licence

GNU General Public License v3.0 uniquement. Voir [LICENSE](LICENSE).
