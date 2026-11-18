const MAX_MESSAGE_BYTES: usize = 2048;
const MASK: &str = "***";
const SECRET_KEYS: [&str; 12] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "api-key",
    "apikey",
    "access_key",
    "private_key",
    "authorization",
    "credential",
];
const SCHEMES: [&str; 3] = ["bearer", "basic", "token"];
const TOKEN_PREFIXES: [(&str, usize); 10] = [
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("glpat-", 10),
    ("xoxb-", 10),
    ("AKIA", 16),
    ("sk-", 20),
];

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '=')
}

fn mask_known_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    'scan: while !rest.is_empty() {
        for (prefix, min) in TOKEN_PREFIXES {
            if let Some(tail) = rest.strip_prefix(prefix) {
                let length = tail.chars().take_while(|c| token_char(*c)).count();
                if length >= min {
                    out.push_str(MASK);
                    rest = &tail[tail
                        .char_indices()
                        .nth(length)
                        .map_or(tail.len(), |(i, _)| i)..];
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn mask_key_values(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for key in SECRET_KEYS {
        let mut from = 0;
        while let Some(found) = lower[from..].find(key) {
            let start = from + found;
            let mut at = start + key.len();
            from = at;
            let bytes = line.as_bytes();
            while at < bytes.len() && matches!(bytes[at], b'"' | b'\'' | b' ' | b'\t') {
                at += 1;
            }
            if at >= bytes.len() || !matches!(bytes[at], b'=' | b':') {
                continue;
            }
            at += 1;
            while at < bytes.len() && matches!(bytes[at], b' ' | b'\t' | b'"' | b'\'') {
                at += 1;
            }
            let mut end = at;
            while end < bytes.len()
                && !matches!(bytes[end], b' ' | b'\t' | b'"' | b'\'' | b',' | b';' | b'&')
            {
                end += 1;
            }
            if end == at {
                continue;
            }
            let word = lower[at..end].to_owned();
            if SCHEMES.contains(&word.as_str()) {
                let mut more = end;
                while more < bytes.len() && bytes[more] == b' ' {
                    more += 1;
                }
                let mut stop = more;
                while stop < bytes.len()
                    && !matches!(bytes[stop], b' ' | b'\t' | b'"' | b'\'' | b',' | b';')
                {
                    stop += 1;
                }
                if stop > more {
                    end = stop;
                }
            }
            spans.push((at, end));
            from = end;
        }
    }
    if spans.is_empty() {
        return line.to_owned();
    }
    spans.sort_unstable();
    let mut out = String::new();
    let mut cursor = 0;
    for (start, end) in spans {
        if start < cursor {
            continue;
        }
        out.push_str(&line[cursor..start]);
        out.push_str(MASK);
        cursor = end;
    }
    out.push_str(&line[cursor..]);
    out
}

fn mask_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_key_block = false;
    for line in text.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            in_key_block = true;
            out.push_str("[clé privée masquée]\n");
            continue;
        }
        if in_key_block {
            if line.contains("-----END") {
                in_key_block = false;
            }
            continue;
        }
        out.push_str(&mask_known_tokens(&mask_key_values(line)));
        out.push('\n');
    }
    out
}

pub fn filter_diagnostic(raw: &[u8]) -> String {
    let masked = mask_secrets(&String::from_utf8_lossy(raw));
    let text = masked;
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
    fn secrets_printed_by_a_failing_tool_are_masked_but_the_cause_stays_readable() {
        let raw = "lint failed: a.txt has trailing spaces\nAPI_TOKEN=s3cretvalue123 loaded\npassword: \"hunter2\"\nAuthorization: Bearer abcdef.ghi-jkl\nusing ghp_abcdefghijklmnopqrstuvwxyz0123 to push\nAKIAIOSFODNN7EXAMPLE leaked\nsecret = 'x y'\n-----BEGIN RSA PRIVATE KEY-----\nMIIEvQIBADANBgkq\n-----END RSA PRIVATE KEY-----\ndone";
        let filtered = filter_diagnostic(raw.as_bytes());
        for leaked in [
            "s3cretvalue123",
            "hunter2",
            "abcdef.ghi-jkl",
            "ghp_abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "MIIEvQIBADANBgkq",
        ] {
            assert!(!filtered.contains(leaked), "{leaked} leaked: {filtered}");
        }
        assert!(filtered.contains("lint failed: a.txt has trailing spaces"));
        assert!(filtered.contains("API_TOKEN=***"));
        assert!(filtered.contains("password: \"***\""));
        assert!(
            filtered.contains("Authorization: Bearer ***")
                || filtered.contains("Authorization: ***")
        );
        assert!(filtered.contains("[clé privée masquée]"));
        assert!(filtered.ends_with("done"));
    }

    #[test]
    fn words_that_merely_look_like_keys_are_left_alone() {
        let raw = "the token was refused\nsee password policy at docs\nsk-short\nkey: value";
        assert_eq!(filter_diagnostic(raw.as_bytes()), raw);
    }

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
