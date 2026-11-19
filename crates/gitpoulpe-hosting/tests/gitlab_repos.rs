mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, MemoryStore, SecretStore, list_organizations, list_repositories,
};

fn connection(api: &FakeApi) -> Connection {
    Connection {
        id: "gl/octo".to_owned(),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", api.base()),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection, token: &str) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), token).unwrap();
    store
}

#[tokio::test]
async fn groups_and_subgroups_are_listed_with_their_full_path() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn, "glgood");
    let groups = list_organizations(&store, &conn).await.unwrap();
    let paths: Vec<&str> = groups.items.iter().map(|g| g.login.as_str()).collect();
    assert_eq!(paths, ["acme", "acme/platform", "acme/platform/infra"]);
    assert_eq!(groups.items[0].description.as_deref(), Some("Acme Inc"));
}

#[tokio::test]
async fn accessible_projects_include_direct_shares_with_the_full_path_permissions_and_pagination() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn, "glgood");
    let all = list_repositories(&store, &conn, None).await.unwrap();
    let names: Vec<String> = all.items.iter().map(|r| r.full_name()).collect();
    assert_eq!(
        names,
        [
            "acme/platform/infra/tool",
            "octo/notes",
            "octo/old",
            "shared/lib"
        ]
    );
    let tool = &all.items[0];
    assert_eq!(
        (tool.owner.as_str(), tool.name.as_str()),
        ("acme/platform/infra", "tool")
    );
    assert!(tool.private && tool.can_push);
    let shared = all.items.iter().find(|r| r.name == "lib").unwrap();
    assert!(!shared.can_push && shared.fork);
    assert_eq!(shared.parent.as_deref(), Some("upstream/lib"));
    assert!(
        !all.items
            .iter()
            .find(|r| r.name == "notes")
            .unwrap()
            .private
    );
    assert!(all.items.iter().find(|r| r.name == "old").unwrap().archived);
    assert!(all.items.iter().all(|r| r.connection_id == "gl/octo"));
    assert_eq!(
        tool.clone_url,
        "https://gitlab.test/acme/platform/infra/tool.git"
    );
    assert!(api.requests().contains("page=2"));
    assert!(api.requests().contains("/projects?membership=true"));
}

#[tokio::test]
async fn a_group_lists_its_subgroups_projects_and_bad_paths_never_reach_the_network() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn, "glgood");
    let group = list_repositories(&store, &conn, Some("acme/platform"))
        .await
        .unwrap();
    let names: Vec<String> = group.items.iter().map(|r| r.full_name()).collect();
    assert_eq!(names, ["acme/platform/api", "acme/platform/infra/tool"]);
    assert!(
        api.requests()
            .contains("/groups/acme%2Fplatform/projects?include_subgroups=true")
    );
    assert_eq!(
        list_repositories(&store, &conn, Some("ghost"))
            .await
            .unwrap_err(),
        GithubError::NotFound
    );
    let before = api.requests();
    for bad in ["", "../x", "a//b", "a b", "a?x=1", "/lead", "a/./b"] {
        assert_eq!(
            list_repositories(&store, &conn, Some(bad))
                .await
                .unwrap_err(),
            GithubError::NotFound,
            "{bad:?}"
        );
    }
    assert_eq!(api.requests(), before);
}

#[tokio::test]
async fn a_rejected_token_is_reported_as_such() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    assert_eq!(
        list_organizations(&stored(&conn, "gone"), &conn)
            .await
            .unwrap_err(),
        GithubError::Unauthorized
    );
    assert_eq!(
        list_repositories(&MemoryStore::default(), &conn, None)
            .await
            .unwrap_err(),
        GithubError::NotConnected
    );
}
