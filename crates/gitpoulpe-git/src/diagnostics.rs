const MAX_MESSAGE_BYTES: usize = 2048;

pub fn filter_diagnostic(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(text.len().min(MAX_MESSAGE_BYTES));
    let mut rest: &str = text.trim();
    while let Some(scheme) = rest.find("://") {
        let (head, tail) = rest.split_at(scheme + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str("***@");
                out.push_str(&tail[at + 1..end]);
            }
            None => out.push_str(&tail[..end]),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    if out.len() > MAX_MESSAGE_BYTES {
        let mut cut = MAX_MESSAGE_BYTES;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_in_urls_are_redacted_but_the_host_is_kept() {
        let raw = b"fatal: unable to access 'https://user:s3cret@example.com/repo.git/': failed";
        let filtered = filter_diagnostic(raw);
        assert!(!filtered.contains("s3cret") && !filtered.contains("user"));
        assert!(filtered.contains("https://***@example.com/repo.git/"));
        assert!(filtered.ends_with("failed"));
    }

    #[test]
    fn plain_messages_and_urls_without_credentials_are_unchanged() {
        let raw = b"fatal: not a git repository (or any of the parent directories): .git\n";
        assert_eq!(
            filter_diagnostic(raw),
            raw.trim_ascii_end()
                .iter()
                .map(|&b| b as char)
                .collect::<String>()
        );
        assert_eq!(
            filter_diagnostic(b"see https://example.com/a"),
            "see https://example.com/a"
        );
    }

    #[test]
    fn long_output_is_truncated_on_a_character_boundary() {
        let raw = "é".repeat(5000);
        let filtered = filter_diagnostic(raw.as_bytes());
        assert!(filtered.len() <= MAX_MESSAGE_BYTES + '…'.len_utf8());
        assert!(filtered.ends_with('…'));
    }
}
