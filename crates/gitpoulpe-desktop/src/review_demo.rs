use std::sync::Arc;

use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    Attachment, AttachmentKind, ChangedFile, Check, CheckState, Checks, FileMedia, IssueComment,
    MergeMethod, MergeReadiness, Preview, PullComments, PullCommit, PullDetail, PullFile,
    PullState, PullSummary, RemoteRef, Review, ReviewComment, Side,
};

use crate::attachments::Media;
use crate::pull_screen::Candidate;
use crate::state::State;

const SITE: &str = "https://github.com";
const SHOT: &str =
    "https://github.com/user-attachments/assets/00000000-0000-4000-8000-000000000001";
const VIDEO: &str =
    "https://github.com/user-attachments/assets/00000000-0000-4000-8000-000000000002";

fn summary(number: u64, title: &str, author: &str, draft: bool, head: &str) -> PullSummary {
    PullSummary {
        number,
        title: title.to_owned(),
        state: PullState::Open,
        draft,
        author: author.to_owned(),
        head_ref: head.to_owned(),
        head_sha: format!("{number:0>7}e4a9c1d2b3f4a5b6c7d8e9f0a1b2c3d4e5f6"),
        base_ref: "staging".to_owned(),
        base_sha: "0a514d7ef59c6357878a244fdf8365d03c504767".to_owned(),
        url: format!("{SITE}/exemple/atelier/pull/{number}"),
        updated_at: "2026-11-24T09:12:00Z".to_owned(),
    }
}

fn attachment(
    body: &str,
    needle: &str,
    kind: AttachmentKind,
    name: &str,
    link: &str,
) -> Attachment {
    let start = body.find(needle).unwrap_or(0);
    Attachment {
        span: start..start + needle.len(),
        kind,
        name: name.to_owned(),
        link: link.to_owned(),
        preview: (kind == AttachmentKind::Image).then(|| Preview::Anonymous(link.to_owned())),
    }
}

fn check(name: &str, state: CheckState) -> Check {
    Check {
        name: name.to_owned(),
        state,
        url: format!("{SITE}/exemple/atelier/actions/runs/1/job/{}", name.len()),
        legacy: false,
    }
}

fn file(path: &str, status: &str, patch: Option<&str>) -> PullFile {
    let (additions, deletions) = patch.map_or((0, 0), |p| {
        (
            p.lines().filter(|l| l.starts_with('+')).count() as u64,
            p.lines().filter(|l| l.starts_with('-')).count() as u64,
        )
    });
    PullFile {
        change: ChangedFile {
            path: path.to_owned(),
            status: status.to_owned(),
            additions,
            deletions,
        },
        patch: patch.map(str::to_owned),
        media: None,
    }
}

fn picture(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let band = (y * 6 / height.max(1)) as u8;
            let edge = x < 4 || y < 4 || x >= width - 4 || y >= height - 4;
            let (r, g, b) = if edge {
                (60, 90, 96)
            } else if y < height / 6 {
                (24, 40, 44)
            } else {
                (30 + band * 6, 52 + band * 4, 58 + (x * 40 / width) as u8)
            };
            bytes.extend_from_slice(&[r, g, b, 255]);
        }
    }
    bytes
}

pub fn review_scene(state: &mut State) {
    let connection = Connection {
        id: "github.com/d-hauet".to_owned(),
        provider: Provider::Github,
        api: "https://api.github.com".to_owned(),
        web: SITE.to_owned(),
        login: "d-hauet".to_owned(),
        scopes: Vec::new(),
    };
    state.pulls.open = true;
    state.pulls.candidates = vec![Candidate {
        connection,
        remote: "origin".to_owned(),
        reference: RemoteRef {
            host: "github.com".to_owned(),
            owner: "exemple".to_owned(),
            repo: "atelier".to_owned(),
        },
    }];
    let list = &mut state.pulls.list;
    list.open = true;
    list.items = vec![
        summary(
            541,
            "[Agent V2] Écrans d'administration de l'analyseur et navigation des réglages",
            "j-lorette",
            true,
            "agent-v2/analyzer-admin",
        ),
        summary(
            543,
            "Parcourir et actualiser les dépôts de l'organisation depuis le panneau latéral",
            "d-hauet",
            false,
            "feature/org-browser",
        ),
        summary(
            542,
            "Classer les brouillons V2 dans BigQuery et exporter les gros fichiers par lots",
            "d-hauet",
            false,
            "feature/ranked-drafts",
        ),
        summary(517, "Agent V2", "d-hauet", true, "feature/agent-v2"),
        summary(
            519,
            "Retirer un échec de filtre en double entre les magasins",
            "d-hauet",
            false,
            "fix/duplicate-filter",
        ),
        summary(
            497,
            "Corréler les origines réseau vérifiées entre les services",
            "d-hauet",
            true,
            "feature/network-origins",
        ),
        summary(
            150,
            "Stockage privé des rapports V2 et qualification des sources",
            "d-hauet",
            true,
            "feature/private-reports",
        ),
    ];
    list.selected = Some(541);
    let body = format!(
        "Ajoute les écrans d'administration de l'analyseur V2 derrière le drapeau `agent_v2_admin`.\n\n\
         - liste des analyseurs avec leur dernier passage ;\n\
         - formulaire de réglage avec validation côté serveur ;\n\
         - lien depuis la navigation principale.\n\n\
         Capture de l'écran de liste :\n\
         <img width=\"1200\" alt=\"Liste des analyseurs\" src=\"{SHOT}\" />\n\n\
         Démonstration du formulaire :\n\n{VIDEO}\n\n\
         À relire en priorité : la migration des réglages existants."
    );
    let attachments = vec![
        attachment(
            &body,
            &format!("<img width=\"1200\" alt=\"Liste des analyseurs\" src=\"{SHOT}\" />"),
            AttachmentKind::Image,
            "Liste des analyseurs",
            SHOT,
        ),
        attachment(
            &body,
            VIDEO,
            AttachmentKind::Video,
            "formulaire-reglages.mov",
            VIDEO,
        ),
    ];
    let mut files = vec![
        file(
            "apps/console/src/analyzer/AdminList.tsx",
            "added",
            Some(
                "@@ -0,0 +1,9 @@\n+export function AdminList({ analyzers }: Props) {\n+  if (analyzers.length === 0) {\n+    return <EmptyState title=\"Aucun analyseur\" />;\n+  }\n+  return (\n+    <Table rows={analyzers} columns={columns} />\n+  );\n+}\n+",
            ),
        ),
        file(
            "apps/console/src/navigation/routes.ts",
            "modified",
            Some(
                "@@ -12,6 +12,8 @@ export const routes = [\n   { path: \"/reports\", element: <Reports /> },\n   { path: \"/sources\", element: <Sources /> },\n-  { path: \"/settings\", element: <Settings /> },\n+  { path: \"/settings\", element: <Settings />, children: settingsRoutes },\n+  { path: \"/admin/analyzers\", element: <AdminList />, flag: \"agent_v2_admin\" },\n+  { path: \"/admin/analyzers/:id\", element: <AdminForm />, flag: \"agent_v2_admin\" },\n ];",
            ),
        ),
        file(
            "services/agent/migrations/0042_analyzer_settings.sql",
            "added",
            Some(
                "@@ -0,0 +1,5 @@\n+alter table analyzer add column settings jsonb not null default '{}';\n+update analyzer set settings = jsonb_build_object('threshold', threshold);\n+alter table analyzer drop column threshold;\n+create index analyzer_settings_mode on analyzer ((settings->>'mode'));\n+",
            ),
        ),
        file("apps/console/public/analyzer-empty.png", "modified", None),
    ];
    files[3].media = Some(FileMedia {
        link: format!("{SITE}/exemple/atelier/blob/head/apps/console/public/analyzer-empty.png"),
        before: Some(Preview::Contents("demo/before".to_owned())),
        after: Some(Preview::Contents("demo/after".to_owned())),
    });
    let detail = PullDetail {
        summary: list.items[0].clone(),
        body,
        attachments,
        commits: vec![
            PullCommit {
                sha: "8e8328d1".to_owned(),
                subject: "Intégrer la navigation d'administration de l'analyseur".to_owned(),
                author: "j-lorette".to_owned(),
            },
            PullCommit {
                sha: "c0d79b84".to_owned(),
                subject: "Ajouter les vues d'administration de l'analyseur V2".to_owned(),
                author: "j-lorette".to_owned(),
            },
            PullCommit {
                sha: "95909d3a".to_owned(),
                subject: "Isoler le front de l'analyseur V2 de la V1".to_owned(),
                author: "j-lorette".to_owned(),
            },
            PullCommit {
                sha: "6746e4a9".to_owned(),
                subject: "Migrer les réglages existants vers settings jsonb".to_owned(),
                author: "j-lorette".to_owned(),
            },
        ],
        files,
        files_truncated: false,
        changed_during_load: false,
    };
    list.detail = Some(detail);
    list.comments = PullComments {
        issue: vec![IssueComment {
            id: 1,
            author: "d-hauet".to_owned(),
            body: "La migration supprime `threshold` dans la même passe : on garde une version de transition ?".to_owned(),
            created_at: "2026-11-23T16:40:00Z".to_owned(),
            attachments: Vec::new(),
        }],
        review: vec![ReviewComment {
            id: 10,
            author: "d-hauet".to_owned(),
            body: "Index utile, mais `settings->>'mode'` n'est pas encore lu ailleurs.".to_owned(),
            path: "services/agent/migrations/0042_analyzer_settings.sql".to_owned(),
            line: Some(4),
            side: Side::Right,
            commit_sha: "6746e4a9".to_owned(),
            in_reply_to: None,
            outdated: false,
            created_at: "2026-11-23T16:42:00Z".to_owned(),
            attachments: Vec::new(),
        }],
    };
    let mut checks = vec![
        check("Console E2E · shard 1/2", CheckState::Failure),
        check("SQL · migrations", CheckState::Failure),
        check("Plan affected work", CheckState::Pending),
    ];
    for name in [
        "Audit · JavaScript dependencies",
        "Audit · Python dependencies",
        "Lint · console",
        "Lint · services",
        "Unit · console",
        "Unit · agent",
        "Unit · renderer",
        "Typecheck",
        "Build · console",
        "Build · agent",
        "Product validation · HTTP contracts",
        "Docs links",
    ] {
        checks.push(check(name, CheckState::Success));
    }
    for i in 0..20 {
        checks.push(check(
            &format!("Python · component {i} · PostgreSQL"),
            CheckState::Skipped,
        ));
    }
    list.checks = Some(Checks {
        sha: "5410000e4a9c1d2b".to_owned(),
        items: checks,
        truncated: false,
    });
    list.merge = Some(MergeReadiness {
        head_sha: "5410000e4a9c1d2b".to_owned(),
        state: PullState::Open,
        draft: true,
        mergeable_state: "draft".to_owned(),
        methods: vec![MergeMethod::Squash, MergeMethod::Merge],
        required_checks: Vec::new(),
        required_approvals: 1,
        approvals: 0,
        protection_unreadable: false,
        blockers: vec![
            "la pull request est un brouillon".to_owned(),
            "2 vérifications obligatoires ont échoué".to_owned(),
        ],
    });
    list.reviews = vec![Review {
        id: 3,
        author: "a-moreau".to_owned(),
        state: "COMMENTED".to_owned(),
        body: "Je regarde le formulaire demain.".to_owned(),
        commit_sha: "95909d3a".to_owned(),
        submitted_at: "2026-11-23T18:05:00Z".to_owned(),
    }];
    let cache = rune::ImageCache::new(Arc::new(|| {}));
    for (key, width, height) in [
        (SHOT, 1200, 640),
        ("demo/before", 320, 200),
        ("demo/after", 320, 200),
    ] {
        let image = cache.rgba(width, height, picture(width, height));
        let ready = image.wait().ok();
        if let Some(size) = ready {
            list.media.insert(key.to_owned(), Media::Ready(image, size));
        }
    }
    state.images = Some(cache);
}

pub fn review_files_scene(state: &mut State) {
    review_scene(state);
    state.set_review_tab(crate::pull_list::ReviewTab::Files);
    state.pull_pick_line(
        "apps/console/src/navigation/routes.ts".to_owned(),
        16,
        Side::Right,
    );
    state.pulls.list.compose =
        rune::TextEdit::new("Le drapeau devrait aussi masquer l'entrée de menu.");
    state.pull_add_to_draft();
    state.pull_pick_line(
        "apps/console/src/analyzer/AdminList.tsx".to_owned(),
        3,
        Side::Right,
    );
    state.pulls.list.compose = rune::TextEdit::new("Un état vide plus explicite ?");
}
