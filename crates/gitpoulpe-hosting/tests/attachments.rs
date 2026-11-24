mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    AttachmentKind, GithubError, MemoryStore, Preview, SecretStore, fetch_preview, list_comments,
    load_pull,
};
use serde_json::Value;

const PNG: &[u8] = b"\x89PNG";

fn connection(api: &FakeApi, provider: Provider) -> Connection {
    let gitlab = provider == Provider::Gitlab;
    Connection {
        id: "one/octo".to_owned(),
        provider,
        api: if gitlab {
            format!("{}/api/v4", api.base())
        } else {
            api.base()
        },
        web: api.base(),
        login: "octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection, token: &str) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), token).unwrap();
    store
}

fn requests(api: &FakeApi) -> Vec<Value> {
    api.requests()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn github_attachments_are_typed_in_place_and_images_are_fetched_without_the_token() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api, Provider::Github);
    let store = stored(&conn, "good");
    let detail = load_pull(&store, &conn, "acme", "proj", 7).await.unwrap();
    let found: Vec<(AttachmentKind, &str)> = detail
        .attachments
        .iter()
        .map(|a| (a.kind, a.name.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (AttachmentKind::Image, "Capture"),
            (AttachmentKind::Video, "demo.mp4"),
            (AttachmentKind::File, "trace.zip"),
            (AttachmentKind::Image, "ext"),
            (AttachmentKind::Image, "gone"),
        ]
    );
    let zip = &detail.attachments[2];
    assert_eq!(
        &detail.body[zip.span.clone()],
        format!(
            "[trace.zip]({}/user-attachments/files/7/trace.zip)",
            api.base()
        )
    );
    for image in [&detail.attachments[0], &detail.attachments[3]] {
        let bytes = fetch_preview(&store, &conn, image.preview.as_ref().unwrap())
            .await
            .unwrap();
        assert!(bytes.starts_with(PNG), "{}", image.name);
    }
    let gone = fetch_preview(
        &store,
        &conn,
        detail.attachments[4].preview.as_ref().unwrap(),
    )
    .await;
    assert_eq!(gone, Err(GithubError::NotFound));

    let shot = detail
        .files
        .iter()
        .find(|f| f.change.path == "docs/shot.png")
        .and_then(|f| f.media.as_ref())
        .unwrap();
    assert_eq!(
        shot.link,
        format!("{}/acme/proj/blob/sha7/docs/shot.png", api.base())
    );
    let before = fetch_preview(&store, &conn, shot.before.as_ref().unwrap())
        .await
        .unwrap();
    let after = fetch_preview(&store, &conn, shot.after.as_ref().unwrap())
        .await
        .unwrap();
    assert!(before.starts_with(PNG) && after.starts_with(PNG) && before != after);
    let video = detail
        .files
        .iter()
        .find(|f| f.change.path == "assets/demo.mp4")
        .and_then(|f| f.media.as_ref())
        .unwrap();
    assert_eq!((&video.before, &video.after), (&None, &None));
    assert!(
        detail
            .files
            .iter()
            .filter(|f| f.patch.is_some())
            .all(|f| f.media.is_none())
    );

    let comments = list_comments(&store, &conn, "acme", "proj", 7)
        .await
        .unwrap();
    assert_eq!(comments.issue[0].attachments[0].kind, AttachmentKind::Image);
    let line = comments.review.iter().find(|c| c.id == 71).unwrap();
    assert_eq!(line.attachments[0].name, "écran");
    assert!(matches!(
        line.attachments[0].preview,
        Some(Preview::Anonymous(_))
    ));

    let log = requests(&api);
    let media: Vec<&Value> = log
        .iter()
        .filter(|e| {
            e["path"]
                .as_str()
                .is_some_and(|p| p.starts_with("/media/") || p.starts_with("/camo/"))
        })
        .collect();
    assert_eq!(
        media.len(),
        4,
        "the redirect from the proxy is followed once"
    );
    assert!(media.iter().all(|e| e["auth"].is_null()));
    for path in [
        "/repos/acme/proj/pulls/7",
        "/repos/acme/proj/issues/7/comments?per_page=100",
        "/repos/acme/proj/pulls/7/comments?per_page=100",
    ] {
        let entry = log.iter().find(|e| e["path"] == path).unwrap();
        assert_eq!(
            entry["accept"], "application/vnd.github.full+json",
            "{path}"
        );
    }
    let contents: Vec<&Value> = log
        .iter()
        .filter(|e| e["path"].as_str().is_some_and(|p| p.contains("/contents/")))
        .collect();
    assert_eq!(contents.len(), 2);
    assert!(contents.iter().all(|e| e["auth"] == "Bearer good"));
}

#[tokio::test]
async fn a_preview_outside_the_provider_hosts_is_refused_before_any_request() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api, Provider::Github);
    let store = stored(&conn, "good");
    let refused = fetch_preview(
        &store,
        &conn,
        &Preview::Anonymous("https://evil.test/x.png".to_owned()),
    )
    .await
    .unwrap_err();
    assert!(
        refused.to_string().contains("hors des serveurs"),
        "{refused}"
    );
    assert!(requests(&api).is_empty());

    let bounced = fetch_preview(
        &store,
        &conn,
        &Preview::Anonymous(format!("{}/redirect", api.base())),
    )
    .await
    .unwrap_err();
    assert!(bounced.to_string().contains("non autorisée"), "{bounced}");
}

#[tokio::test]
async fn gitlab_uploads_go_through_the_api_with_the_token_and_a_redirect_drops_it() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api, Provider::Gitlab);
    let store = stored(&conn, "glgood");
    let detail = load_pull(&store, &conn, "acme/platform", "api", 50)
        .await
        .unwrap();
    let found: Vec<(AttachmentKind, &str)> = detail
        .attachments
        .iter()
        .map(|a| (a.kind, a.name.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (AttachmentKind::Image, "écran"),
            (AttachmentKind::Image, "déplacée"),
            (AttachmentKind::File, "journal"),
        ]
    );
    assert_eq!(
        detail.attachments[2].link,
        format!(
            "{}/acme/platform/api/uploads/0123456789abcdef0123456789abcdef/run.log",
            api.base()
        )
    );
    for image in &detail.attachments[..2] {
        let bytes = fetch_preview(&store, &conn, image.preview.as_ref().unwrap())
            .await
            .unwrap();
        assert!(bytes.starts_with(PNG), "{}", image.name);
    }
    let file = |path: &str| {
        detail
            .files
            .iter()
            .find(|f| f.change.path == path)
            .unwrap_or_else(|| panic!("{path}"))
    };
    assert!(file("src/a.rs").media.is_none());
    let shot = file("docs/shot.png").media.as_ref().unwrap();
    for version in [&shot.before, &shot.after] {
        let bytes = fetch_preview(&store, &conn, version.as_ref().unwrap())
            .await
            .unwrap();
        assert!(bytes.starts_with(PNG));
    }
    let tool = file("bin/tool").media.as_ref().unwrap();
    assert_eq!(
        tool.link,
        format!(
            "{}/acme/platform/api/-/blob/{}/bin/tool",
            api.base(),
            "b".repeat(40)
        )
    );
    assert_eq!((&tool.before, &tool.after), (&None, &None));

    let comments = list_comments(&store, &conn, "acme/platform", "api", 50)
        .await
        .unwrap();
    assert_eq!(comments.issue[0].attachments[0].kind, AttachmentKind::Image);

    let log = requests(&api);
    let upload = log
        .iter()
        .find(|e| {
            e["path"]
                .as_str()
                .is_some_and(|p| p.ends_with("/screen%20shot.png"))
        })
        .unwrap();
    assert_eq!(upload["auth"], "Bearer glgood");
    let storage = log
        .iter()
        .find(|e| e["path"] == "/storage/moved.png")
        .unwrap();
    assert_eq!(storage["auth"], "");
}
