use std::fmt;
use std::path::PathBuf;

use gitpoulpe_core::{DiffLine, DiffLineKind, Hunk};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffParseError(String);

impl fmt::Display for DiffParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DiffParseError {}

fn fail<T>(message: impl Into<String>) -> Result<T, DiffParseError> {
    Err(DiffParseError(message.into()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub binary: bool,
    pub mode_change: Option<(String, String)>,
    pub rename: Option<(PathBuf, PathBuf)>,
    pub type_changed: bool,
    pub hunks: Vec<Hunk>,
    pub lines: u64,
}

#[derive(Debug, Default)]
pub struct DiffParser {
    buffer: Vec<u8>,
    in_hunk: Option<(u32, u32)>,
    after_hunk: bool,
    old_number: u32,
    new_number: u32,
    lines: u64,
    hunks: Vec<Hunk>,
    binary: bool,
    old_mode: Option<String>,
    new_mode: Option<String>,
    rename_from: Option<PathBuf>,
    rename_to: Option<PathBuf>,
    sections: u32,
}

fn parse_range(token: &[u8], sign: u8) -> Option<(u32, u32)> {
    let rest = token.strip_prefix(&[sign])?;
    let text = std::str::from_utf8(rest).ok()?;
    match text.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

impl DiffParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn line_count(&self) -> u64 {
        self.lines
    }

    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<DiffLine>) -> Result<(), DiffParseError> {
        self.buffer.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(offset) = self.buffer[start..].iter().position(|&b| b == b'\n') {
            let end = start + offset;
            let line = self.buffer[start..end].to_vec();
            start = end + 1;
            self.line(&line, out)?;
        }
        self.buffer.drain(..start);
        Ok(())
    }

    pub fn finish(self) -> Result<DiffSummary, DiffParseError> {
        if !self.buffer.is_empty() {
            return fail("the diff ended in the middle of a line");
        }
        if matches!(self.in_hunk, Some((old, new)) if old > 0 || new > 0) {
            return fail("the diff ended in the middle of a hunk");
        }
        Ok(DiffSummary {
            binary: self.binary,
            mode_change: self.old_mode.zip(self.new_mode),
            rename: self.rename_from.zip(self.rename_to),
            type_changed: self.sections > 1,
            hunks: self.hunks,
            lines: self.lines,
        })
    }

    fn emit(&mut self, line: DiffLine, out: &mut Vec<DiffLine>) {
        if let Some(hunk) = self.hunks.last_mut() {
            hunk.line_count += 1;
        }
        self.lines += 1;
        out.push(line);
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<DiffLine>) -> Result<(), DiffParseError> {
        if let Some((old_left, new_left)) = self.in_hunk.filter(|&(o, n)| o > 0 || n > 0) {
            return self.body_line(line, old_left, new_left, out);
        }
        if line.first() == Some(&b'\\') && self.after_hunk {
            self.emit(marker(), out);
            return Ok(());
        }
        if self.after_hunk && stray_body_line(line) {
            return fail("a hunk has more lines than its header announces");
        }
        self.after_hunk = false;
        if line.starts_with(b"@@ ") {
            return self.hunk_header(line, out);
        }
        self.header_line(line);
        Ok(())
    }

    fn header_line(&mut self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        if text.starts_with("Binary files ") || text.starts_with("GIT binary patch") {
            self.binary = true;
        } else if text.starts_with("diff --git ") {
            self.sections += 1;
        } else if let Some(mode) = text.strip_prefix("old mode ") {
            self.old_mode = Some(mode.to_owned());
        } else if let Some(mode) = text.strip_prefix("new mode ") {
            self.new_mode = Some(mode.to_owned());
        } else if line.starts_with(b"rename from ") {
            self.rename_from = Some(path_of(&line[b"rename from ".len()..]));
        } else if line.starts_with(b"rename to ") {
            self.rename_to = Some(path_of(&line[b"rename to ".len()..]));
        }
    }

    fn hunk_header(&mut self, line: &[u8], out: &mut Vec<DiffLine>) -> Result<(), DiffParseError> {
        let mut tokens = line.split(|&b| b == b' ').skip(1);
        let old = tokens.next().and_then(|t| parse_range(t, b'-'));
        let new = tokens.next().and_then(|t| parse_range(t, b'+'));
        let (Some((old_start, old_len)), Some((new_start, new_len))) = (old, new) else {
            return fail("malformed hunk header");
        };
        self.hunks.push(Hunk {
            old_start,
            old_len,
            new_start,
            new_len,
            first_line: self.lines,
            line_count: 0,
        });
        self.in_hunk = Some((old_len, new_len));
        self.after_hunk = old_len == 0 && new_len == 0;
        self.old_number = old_start;
        self.new_number = new_start;
        self.emit(
            DiffLine {
                kind: DiffLineKind::HunkHeader,
                old_number: None,
                new_number: None,
                text: line.to_vec(),
            },
            out,
        );
        Ok(())
    }

    fn body_line(
        &mut self,
        line: &[u8],
        old_left: u32,
        new_left: u32,
        out: &mut Vec<DiffLine>,
    ) -> Result<(), DiffParseError> {
        let (kind, text) = match line.first() {
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            None => (DiffLineKind::Context, line),
            Some(b'-') => (DiffLineKind::Removed, &line[1..]),
            Some(b'+') => (DiffLineKind::Added, &line[1..]),
            Some(b'\\') => {
                self.emit(marker(), out);
                return Ok(());
            }
            Some(_) => return fail("unexpected line inside a hunk"),
        };
        let consumes_old = kind != DiffLineKind::Added;
        let consumes_new = kind != DiffLineKind::Removed;
        if (consumes_old && old_left == 0) || (consumes_new && new_left == 0) {
            return fail("a hunk has more lines than its header announces");
        }
        let old_number = consumes_old.then_some(self.old_number);
        let new_number = consumes_new.then_some(self.new_number);
        if consumes_old {
            self.old_number += 1;
        }
        if consumes_new {
            self.new_number += 1;
        }
        let remaining = (
            old_left - u32::from(consumes_old),
            new_left - u32::from(consumes_new),
        );
        self.in_hunk = Some(remaining);
        self.after_hunk = remaining == (0, 0);
        self.emit(
            DiffLine {
                kind,
                old_number,
                new_number,
                text: text.to_vec(),
            },
            out,
        );
        Ok(())
    }
}

fn stray_body_line(line: &[u8]) -> bool {
    match line.first() {
        Some(b' ') => true,
        Some(b'+') => !line.starts_with(b"+++ "),
        Some(b'-') => !line.starts_with(b"--- "),
        _ => false,
    }
}

fn marker() -> DiffLine {
    DiffLine {
        kind: DiffLineKind::NoNewlineAtEof,
        old_number: None,
        new_number: None,
        text: Vec::new(),
    }
}

fn path_of(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8], chunk: usize) -> Result<(Vec<DiffLine>, DiffSummary), DiffParseError> {
        let mut parser = DiffParser::new();
        let mut lines = Vec::new();
        for part in bytes.chunks(chunk) {
            parser.push(part, &mut lines)?;
        }
        Ok((lines, parser.finish()?))
    }

    const SAMPLE: &[u8] = b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n\
@@ -1,3 +1,3 @@ fn main\n keep\n--- not a header\n+++ nor this\n tail\n\
@@ -10 +10,2 @@\n-old\n\\ No newline at end of file\n+new\n+more\n";

    #[test]
    fn lines_numbers_and_hunks_do_not_depend_on_chunking() {
        let (whole, summary) = parse(SAMPLE, SAMPLE.len()).unwrap();
        for chunk in [1, 3, 17] {
            assert_eq!(parse(SAMPLE, chunk).unwrap().0, whole, "chunk size {chunk}");
        }
        assert_eq!(summary.hunks.len(), 2);
        assert_eq!(
            (summary.hunks[0].first_line, summary.hunks[0].line_count),
            (0, 5)
        );
        assert_eq!(
            (summary.hunks[1].first_line, summary.hunks[1].line_count),
            (5, 5)
        );
        assert_eq!(summary.lines, 10);
        assert_eq!(whole[2].kind, DiffLineKind::Removed);
        assert_eq!(whole[2].text, b"-- not a header");
        assert_eq!((whole[2].old_number, whole[2].new_number), (Some(2), None));
        assert_eq!(whole[3].text, b"++ nor this");
        assert_eq!(
            (whole[4].old_number, whole[4].new_number),
            (Some(3), Some(3))
        );
        assert_eq!(whole[7].kind, DiffLineKind::NoNewlineAtEof);
        assert_eq!((whole[8].old_number, whole[8].new_number), (None, Some(10)));
    }

    #[test]
    fn binary_mode_and_rename_headers_are_recognised() {
        let (lines, summary) =
            parse(b"diff --git a/b b/b\nBinary files a/b and b/b differ\n", 4).unwrap();
        assert!(lines.is_empty() && summary.binary);
        let (_, summary) =
            parse(b"diff --git a/f b/f\nold mode 100644\nnew mode 100755\n", 5).unwrap();
        assert_eq!(
            summary.mode_change,
            Some(("100644".to_owned(), "100755".to_owned()))
        );
        let (_, summary) = parse(
            b"diff --git a/a b/b\nsimilarity index 100%\nrename from a\nrename to b\n",
            7,
        )
        .unwrap();
        assert_eq!(
            summary.rename,
            Some((PathBuf::from("a"), PathBuf::from("b")))
        );
    }

    #[test]
    fn two_sections_mean_a_type_change() {
        let (_, summary) = parse(
            b"diff --git a/f b/f\n@@ -1 +0,0 @@\n-a\ndiff --git a/f b/f\n@@ -0,0 +1 @@\n+b\n",
            6,
        )
        .unwrap();
        assert!(summary.type_changed);
        assert_eq!(summary.hunks.len(), 2);
    }

    #[test]
    fn crlf_and_non_utf8_bytes_are_kept_in_the_text() {
        let (lines, _) = parse(b"@@ -0,0 +1,2 @@\n+a\r\n+\xff\xfe\n", 3).unwrap();
        assert_eq!(lines[1].text, b"a\r");
        assert_eq!(lines[2].text, b"\xff\xfe");
    }

    #[test]
    fn malformed_or_truncated_diffs_are_errors() {
        assert!(parse(b"@@ -1,2 +1,2 @@\n a\n", 4).is_err());
        assert!(parse(b"@@ nonsense @@\n", 4).is_err());
        assert!(parse(b"@@ -1 +1 @@\n-a\n+b\nextra garbage\n@@ -1 +1 @@\nzzz\n", 4).is_err());
        assert!(parse(b"@@ -1 +1 @@\n a\n b\n", 4).is_err());
        assert!(parse(b"@@ -1 +1 @@\n-a", 4).is_err());
    }
}
