use gitpoulpe_hosting::{RemoteRef, parse_remote};

fn r(host: &str, owner: &str, repo: &str) -> Option<RemoteRef> {
    Some(RemoteRef {
        host: host.to_owned(),
        owner: owner.to_owned(),
        repo: repo.to_owned(),
    })
}

#[test]
fn https_scp_and_ssh_forms_resolve_to_the_same_host_owner_and_repository() {
    for url in [
        "https://github.com/Acme/tool.git",
        "https://token@github.com/Acme/tool",
        "git@github.com:Acme/tool.git",
        "ssh://git@github.com:22/Acme/tool.git",
        "  https://GitHub.com/Acme/tool/ ",
    ] {
        assert_eq!(parse_remote(url), r("github.com", "Acme", "tool"), "{url}");
    }
    assert_eq!(
        parse_remote("https://ghe.example.org:8443/team/app.git"),
        r("ghe.example.org:8443", "team", "app")
    );
}

#[test]
fn local_paths_and_malformed_urls_are_not_taken_for_hosted_repositories() {
    for url in [
        "",
        "/srv/git/tool.git",
        "../tool",
        "C:/x/y",
        "file:///srv/tool.git",
        "https://github.com/onlyowner",
        "https://github.com/a//b",
        "https://github.com//b",
        "https:///a/b",
        "git@:a/b",
        "ftp://h/a/b",
    ] {
        assert_eq!(parse_remote(url), None, "{url}");
    }
}

#[test]
fn nested_namespaces_keep_every_group_in_the_owner() {
    assert_eq!(
        parse_remote("https://gitlab.com/acme/platform/infra/tool.git"),
        r("gitlab.com", "acme/platform/infra", "tool")
    );
    assert_eq!(
        parse_remote("git@gitlab.com:acme/platform/tool.git"),
        r("gitlab.com", "acme/platform", "tool")
    );
}
