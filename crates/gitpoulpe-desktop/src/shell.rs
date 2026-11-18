pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn restore_tag(name: &str, object: &str) -> String {
    format!(
        "git update-ref {} {object}",
        quote(&format!("refs/tags/{name}"))
    )
}

pub fn restore_branch(name: &str, tip: &str) -> String {
    format!("git branch -- {} {tip}", quote(name))
}

pub fn restore_stash(message: &str, oid: &str) -> String {
    format!("git stash store -m {} {oid}", quote(message))
}

pub fn restore_remote(name: &str, url: &str) -> String {
    if gitpoulpe_core::url_has_secret(url) {
        return format!(
            "git remote add -- {} <URL avec identifiants, non affichée>",
            quote(name)
        );
    }
    format!("git remote add -- {} {}", quote(name), quote(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn a_quoted_name_reaches_the_shell_as_one_inert_word() {
        for name in [
            "plain",
            "a b",
            "it's",
            "$(touch /tmp/pwned-gitpoulpe)",
            "`id`",
            "a;b|c&d",
            "x\\y",
            "*",
        ] {
            let out = Command::new("sh")
                .args(["-c", &format!("printf %s {}", quote(name))])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), name);
        }
        assert!(!std::path::Path::new("/tmp/pwned-gitpoulpe").exists());
        assert_eq!(
            restore_tag("v1", "abc"),
            "git update-ref 'refs/tags/v1' abc"
        );
        assert_eq!(restore_branch("x$y", "abc"), "git branch -- 'x$y' abc");
    }
}
