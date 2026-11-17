use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use gitpoulpe_core::{
    BranchInfo, ChangeEntry, EntryKind, Generation, RepoId, SessionId, StatusCode, StatusSnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

fn fail<T>(message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError(message.into()))
}

fn native_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[derive(Debug, Default)]
pub struct StatusParser {
    buffer: Vec<u8>,
    branch: BranchInfo,
    entries: Vec<ChangeEntry>,
    awaiting_original: Option<ChangeEntry>,
}

impl StatusParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), ParseError> {
        self.buffer.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(offset) = self.buffer[start..].iter().position(|&b| b == 0) {
            let end = start + offset;
            let record = self.buffer[start..end].to_vec();
            start = end + 1;
            self.record(&record)?;
        }
        self.buffer.drain(..start);
        Ok(())
    }

    pub fn finish(
        self,
        repo: RepoId,
        session: SessionId,
        generation: Generation,
    ) -> Result<StatusSnapshot, ParseError> {
        if self.awaiting_original.is_some() || !self.buffer.is_empty() {
            return fail("the output ended in the middle of a record");
        }
        Ok(StatusSnapshot {
            repo,
            session,
            generation,
            branch: self.branch,
            entries: self.entries,
            stamps: Default::default(),
        })
    }

    fn record(&mut self, record: &[u8]) -> Result<(), ParseError> {
        if let Some(mut entry) = self.awaiting_original.take() {
            entry.original_path = Some(native_path(record));
            self.entries.push(entry);
            return Ok(());
        }
        match record.first() {
            Some(b'#') => self.header(record),
            Some(b'1') => self.ordinary(record),
            Some(b'2') => self.renamed(record),
            Some(b'u') => self.unmerged(record),
            Some(b'?') => self.simple(record, EntryKind::Untracked, StatusCode::Untracked),
            Some(b'!') => self.simple(record, EntryKind::Ignored, StatusCode::Ignored),
            Some(other) => fail(format!("unknown record type {:?}", *other as char)),
            None => fail("empty record"),
        }
    }

    fn header(&mut self, record: &[u8]) -> Result<(), ParseError> {
        let text = String::from_utf8_lossy(record);
        if let Some(value) = text.strip_prefix("# branch.oid ") {
            self.branch.oid = Some(value.to_owned());
        } else if let Some(value) = text.strip_prefix("# branch.head ") {
            self.branch.head = Some(value.to_owned());
        }
        Ok(())
    }

    fn codes(field: &[u8]) -> Result<(StatusCode, StatusCode), ParseError> {
        match field {
            [x, y] => match (StatusCode::from_byte(*x), StatusCode::from_byte(*y)) {
                (Some(index), Some(worktree)) => Ok((index, worktree)),
                _ => fail(format!(
                    "unknown status code {:?}",
                    String::from_utf8_lossy(field)
                )),
            },
            _ => fail("malformed status code"),
        }
    }

    fn fields(record: &[u8], count: usize) -> Result<Vec<&[u8]>, ParseError> {
        let fields: Vec<&[u8]> = record.splitn(count, |&b| b == b' ').collect();
        if fields.len() != count || fields[count - 1].is_empty() {
            return fail("record has too few fields");
        }
        Ok(fields)
    }

    fn ordinary(&mut self, record: &[u8]) -> Result<(), ParseError> {
        let fields = Self::fields(record, 9)?;
        let (index, worktree) = Self::codes(fields[1])?;
        self.entries.push(ChangeEntry {
            path: native_path(fields[8]),
            original_path: None,
            kind: EntryKind::Ordinary,
            index,
            worktree,
        });
        Ok(())
    }

    fn renamed(&mut self, record: &[u8]) -> Result<(), ParseError> {
        let fields = Self::fields(record, 10)?;
        let (index, worktree) = Self::codes(fields[1])?;
        let kind = match fields[8].first() {
            Some(b'R') => EntryKind::Renamed,
            Some(b'C') => EntryKind::Copied,
            _ => return fail("unknown rename score"),
        };
        self.awaiting_original = Some(ChangeEntry {
            path: native_path(fields[9]),
            original_path: None,
            kind,
            index,
            worktree,
        });
        Ok(())
    }

    fn unmerged(&mut self, record: &[u8]) -> Result<(), ParseError> {
        let fields = Self::fields(record, 11)?;
        let (index, worktree) = Self::codes(fields[1])?;
        self.entries.push(ChangeEntry {
            path: native_path(fields[10]),
            original_path: None,
            kind: EntryKind::Unmerged,
            index,
            worktree,
        });
        Ok(())
    }

    fn simple(
        &mut self,
        record: &[u8],
        kind: EntryKind,
        code: StatusCode,
    ) -> Result<(), ParseError> {
        match record {
            [_, b' ', path @ ..] if !path.is_empty() => {
                self.entries.push(ChangeEntry {
                    path: native_path(path),
                    original_path: None,
                    kind,
                    index: StatusCode::Unmodified,
                    worktree: code,
                });
                Ok(())
            }
            _ => fail("malformed path record"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn parse(bytes: &[u8], chunk: usize) -> Result<StatusSnapshot, ParseError> {
        let mut parser = StatusParser::new();
        for part in bytes.chunks(chunk) {
            parser.push(part)?;
        }
        parser.finish(
            RepoId::from_common_dir(Path::new("/x/.git")),
            SessionId::next(),
            Generation::default(),
        )
    }

    const SAMPLE: &[u8] = b"# branch.oid abc\0# branch.head main\0\
1 .M N... 100644 100644 100644 aaa bbb dir with space/a b.txt\0\
2 R. N... 100644 100644 100644 aaa bbb R100 new name\0old\nname\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? -dash\0";

    #[test]
    fn records_are_parsed_identically_whatever_the_chunking() {
        let whole = parse(SAMPLE, SAMPLE.len()).unwrap();
        for chunk in [1, 2, 7, 64] {
            assert_eq!(
                parse(SAMPLE, chunk).unwrap().entries,
                whole.entries,
                "chunk size {chunk}"
            );
        }
        assert_eq!(whole.branch.head.as_deref(), Some("main"));
        assert_eq!(whole.entries.len(), 4);
        assert_eq!(whole.entries[0].path, Path::new("dir with space/a b.txt"));
        assert_eq!(whole.entries[1].kind, EntryKind::Renamed);
        assert_eq!(
            whole.entries[1].original_path.as_deref(),
            Some(Path::new("old\nname"))
        );
        assert!(whole.entries[2].is_conflict());
        assert_eq!(whole.entries[3].path, Path::new("-dash"));
    }

    #[test]
    fn non_utf8_bytes_survive_in_native_paths() {
        let snapshot = parse(b"? bad\xff\xfename\0", 3).unwrap();
        assert_eq!(
            snapshot.entries[0].path.as_os_str().as_encoded_bytes(),
            b"bad\xff\xfename"
        );
    }

    #[test]
    fn truncated_or_unknown_output_is_an_error_not_a_clean_state() {
        assert!(parse(b"2 R. N... 100644 100644 100644 a b R100 new\0", 5).is_err());
        assert!(parse(b"? partial", 4).is_err());
        assert!(parse(b"x what\0", 4).is_err());
        assert!(parse(b"1 ZZ N... 100644 100644 100644 a b path\0", 4).is_err());
    }
}
