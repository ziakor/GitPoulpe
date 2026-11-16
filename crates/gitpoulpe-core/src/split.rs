use std::ops::Range;

use crate::diff::{DiffLine, DiffLineKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitUnit {
    Linear {
        line_start: u64,
        split_start: u64,
        count: u64,
    },
    Change {
        line_start: u64,
        split_start: u64,
        removed: u64,
        added: u64,
        marker_left: bool,
        marker_right: bool,
    },
}

impl SplitUnit {
    fn split_start(&self) -> u64 {
        match self {
            Self::Linear { split_start, .. } | Self::Change { split_start, .. } => *split_start,
        }
    }

    fn line_start(&self) -> u64 {
        match self {
            Self::Linear { line_start, .. } | Self::Change { line_start, .. } => *line_start,
        }
    }

    pub fn rows(&self) -> u64 {
        match self {
            Self::Linear { count, .. } => *count,
            Self::Change {
                removed,
                added,
                marker_left,
                marker_right,
                ..
            } => (*removed).max(*added) + u64::from(*marker_left || *marker_right),
        }
    }

    pub fn lines(&self) -> u64 {
        match self {
            Self::Linear { count, .. } => *count,
            Self::Change {
                removed,
                added,
                marker_left,
                marker_right,
                ..
            } => removed + added + u64::from(*marker_left) + u64::from(*marker_right),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitCells {
    pub left: Option<u64>,
    pub right: Option<u64>,
    pub full_width: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitIndex {
    units: Vec<SplitUnit>,
    total_lines: u64,
    total_rows: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Removed,
    MarkerLeft,
    Added,
    MarkerRight,
}

struct OpenChange {
    line_start: u64,
    split_start: u64,
    removed: u64,
    added: u64,
    marker_left: bool,
    marker_right: bool,
    phase: Phase,
}

#[derive(Default)]
pub struct SplitIndexBuilder {
    units: Vec<SplitUnit>,
    lines: u64,
    rows: u64,
    open: Option<OpenChange>,
    headers: u64,
}

impl SplitIndexBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, kind: DiffLineKind) {
        match kind {
            DiffLineKind::HunkHeader | DiffLineKind::Context => {
                self.close();
                self.linear();
                if kind == DiffLineKind::HunkHeader {
                    self.headers += 1;
                }
            }
            DiffLineKind::Removed => {
                if self
                    .open
                    .as_ref()
                    .is_some_and(|c| c.phase != Phase::Removed)
                {
                    self.close();
                }
                self.change().removed += 1;
            }
            DiffLineKind::Added => {
                let change = self.change();
                change.added += 1;
                change.phase = Phase::Added;
            }
            DiffLineKind::NoNewlineAtEof => match self.open.as_mut() {
                Some(change)
                    if change.phase == Phase::Removed
                        && !change.marker_left
                        && change.added == 0 =>
                {
                    change.marker_left = true;
                    change.phase = Phase::MarkerLeft;
                }
                Some(change) if change.added > 0 && !change.marker_right => {
                    change.marker_right = true;
                    change.phase = Phase::MarkerRight;
                }
                _ => {
                    self.close();
                    self.linear();
                }
            },
        }
        self.lines += 1;
    }

    fn change(&mut self) -> &mut OpenChange {
        if self.open.is_none() {
            self.open = Some(OpenChange {
                line_start: self.lines,
                split_start: self.rows,
                removed: 0,
                added: 0,
                marker_left: false,
                marker_right: false,
                phase: Phase::Removed,
            });
        }
        self.open.as_mut().expect("a change is open")
    }

    fn linear(&mut self) {
        let contiguous = self.open.is_none() && self.rows == self.units_end_row();
        match self.units.last_mut() {
            Some(SplitUnit::Linear { count, .. }) if contiguous => *count += 1,
            _ => self.units.push(SplitUnit::Linear {
                line_start: self.lines,
                split_start: self.rows,
                count: 1,
            }),
        }
        self.rows += 1;
    }

    fn units_end_row(&self) -> u64 {
        self.units.last().map_or(0, |u| u.split_start() + u.rows())
    }

    fn close(&mut self) {
        if let Some(c) = self.open.take() {
            let unit = SplitUnit::Change {
                line_start: c.line_start,
                split_start: c.split_start,
                removed: c.removed,
                added: c.added,
                marker_left: c.marker_left,
                marker_right: c.marker_right,
            };
            self.rows += unit.rows();
            self.units.push(unit);
        }
    }

    pub fn finish(mut self) -> SplitIndex {
        self.close();
        SplitIndex {
            units: self.units,
            total_lines: self.lines,
            total_rows: self.rows,
        }
    }
}

impl SplitIndex {
    pub fn from_kinds(kinds: impl IntoIterator<Item = DiffLineKind>) -> Self {
        let mut builder = SplitIndexBuilder::new();
        for kind in kinds {
            builder.push(kind);
        }
        builder.finish()
    }

    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    pub fn total_lines(&self) -> u64 {
        self.total_lines
    }

    pub fn units(&self) -> &[SplitUnit] {
        &self.units
    }

    fn unit_of_row(&self, row: u64) -> Option<&SplitUnit> {
        if row >= self.total_rows {
            return None;
        }
        let index = self
            .units
            .partition_point(|u| u.split_start() <= row)
            .checked_sub(1)?;
        self.units.get(index)
    }

    pub fn cells(&self, row: u64) -> Option<SplitCells> {
        let unit = self.unit_of_row(row)?;
        let offset = row - unit.split_start();
        Some(match *unit {
            SplitUnit::Linear { line_start, .. } => SplitCells {
                left: Some(line_start + offset),
                right: Some(line_start + offset),
                full_width: true,
            },
            SplitUnit::Change {
                line_start,
                removed,
                added,
                marker_left,
                marker_right,
                ..
            } => {
                let paired = removed.max(added);
                let after_left = removed + u64::from(marker_left);
                if offset < paired {
                    SplitCells {
                        left: (offset < removed).then_some(line_start + offset),
                        right: (offset < added).then_some(line_start + after_left + offset),
                        full_width: false,
                    }
                } else {
                    SplitCells {
                        left: marker_left.then_some(line_start + removed),
                        right: marker_right.then_some(line_start + after_left + added),
                        full_width: false,
                    }
                }
            }
        })
    }

    pub fn line_ranges(&self, rows: Range<u64>) -> Vec<Range<u64>> {
        let mut ranges: Vec<Range<u64>> = Vec::new();
        let mut push = |range: Range<u64>| {
            if !range.is_empty() {
                ranges.push(range);
            }
        };
        let rows = rows.start..rows.end.min(self.total_rows);
        if rows.is_empty() {
            return ranges;
        }
        let first = self
            .units
            .partition_point(|u| u.split_start() + u.rows() <= rows.start);
        for unit in &self.units[first..] {
            if unit.split_start() >= rows.end {
                break;
            }
            let from = rows.start.max(unit.split_start()) - unit.split_start();
            let to = rows.end.min(unit.split_start() + unit.rows()) - unit.split_start();
            match *unit {
                SplitUnit::Linear { line_start, .. } => push(line_start + from..line_start + to),
                SplitUnit::Change {
                    line_start,
                    removed,
                    added,
                    marker_left,
                    marker_right,
                    ..
                } => {
                    let paired = removed.max(added);
                    let after_left = removed + u64::from(marker_left);
                    push(line_start + from.min(removed)..line_start + to.min(removed));
                    push(
                        line_start + after_left + from.min(added)
                            ..line_start + after_left + to.min(added),
                    );
                    if to > paired {
                        if marker_left {
                            push(line_start + removed..line_start + removed + 1);
                        }
                        if marker_right {
                            push(
                                line_start + after_left + added
                                    ..line_start + after_left + added + 1,
                            );
                        }
                    }
                }
            }
        }
        ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<u64>> = Vec::new();
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    pub fn row_of_line(&self, line: u64) -> Option<u64> {
        if line >= self.total_lines {
            return None;
        }
        let index = self
            .units
            .partition_point(|u| u.line_start() <= line)
            .checked_sub(1)?;
        let unit = self.units.get(index)?;
        let offset = line - unit.line_start();
        Some(match *unit {
            SplitUnit::Linear { split_start, .. } => split_start + offset,
            SplitUnit::Change {
                split_start,
                removed,
                added,
                marker_left,
                ..
            } => {
                let paired = removed.max(added);
                let after_left = removed + u64::from(marker_left);
                let row = if offset < removed {
                    offset
                } else if marker_left && offset == removed {
                    paired
                } else if offset < after_left + added {
                    offset - after_left
                } else {
                    paired
                };
                split_start + row
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitRow {
    Header(usize),
    Pair {
        left: Option<usize>,
        right: Option<usize>,
    },
}

pub fn split_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let index = SplitIndex::from_kinds(lines.iter().map(|l| l.kind));
    (0..index.total_rows())
        .filter_map(|row| {
            let cells = index.cells(row)?;
            let (left, right) = (
                cells.left.map(|l| l as usize),
                cells.right.map(|r| r as usize),
            );
            Some(match (cells.full_width, left) {
                (true, Some(line)) if lines[line].kind == DiffLineKind::HunkHeader => {
                    SplitRow::Header(line)
                }
                _ => SplitRow::Pair { left, right },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use DiffLineKind::*;

    use super::*;

    fn line(kind: DiffLineKind) -> DiffLine {
        DiffLine {
            kind,
            old_number: None,
            new_number: None,
            text: Vec::new(),
        }
    }

    fn lines(kinds: &[DiffLineKind]) -> Vec<DiffLine> {
        kinds.iter().copied().map(line).collect()
    }

    #[test]
    fn removed_and_added_blocks_are_paired_in_order_with_leftovers_alone() {
        let rows = split_rows(&lines(&[
            HunkHeader, Context, Removed, Removed, Removed, Added, Context,
        ]));
        assert_eq!(
            rows,
            vec![
                SplitRow::Header(0),
                SplitRow::Pair {
                    left: Some(1),
                    right: Some(1)
                },
                SplitRow::Pair {
                    left: Some(2),
                    right: Some(5)
                },
                SplitRow::Pair {
                    left: Some(3),
                    right: None
                },
                SplitRow::Pair {
                    left: Some(4),
                    right: None
                },
                SplitRow::Pair {
                    left: Some(6),
                    right: Some(6)
                },
            ]
        );
    }

    #[test]
    fn missing_newline_markers_close_the_block_on_the_side_they_belong_to() {
        let rows = split_rows(&lines(&[Removed, NoNewlineAtEof, Added, NoNewlineAtEof]));
        assert_eq!(
            rows,
            vec![
                SplitRow::Pair {
                    left: Some(0),
                    right: Some(2)
                },
                SplitRow::Pair {
                    left: Some(1),
                    right: Some(3)
                },
            ]
        );
        let rows = split_rows(&lines(&[Removed, NoNewlineAtEof, Added]));
        assert_eq!(
            rows,
            vec![
                SplitRow::Pair {
                    left: Some(0),
                    right: Some(2)
                },
                SplitRow::Pair {
                    left: Some(1),
                    right: None
                },
            ]
        );
        let rows = split_rows(&lines(&[Context, NoNewlineAtEof]));
        assert_eq!(
            rows,
            vec![
                SplitRow::Pair {
                    left: Some(0),
                    right: Some(0)
                },
                SplitRow::Pair {
                    left: Some(1),
                    right: Some(1)
                }
            ]
        );
    }

    #[test]
    fn every_line_appears_in_exactly_one_row_for_any_sequence() {
        let kinds = [
            HunkHeader,
            Context,
            Removed,
            Added,
            Added,
            NoNewlineAtEof,
            Context,
            Removed,
            Removed,
            Context,
        ];
        let all = lines(&kinds);
        let mut seen = vec![0u32; all.len()];
        for row in split_rows(&all) {
            match row {
                SplitRow::Header(i) => seen[i] += 1,
                SplitRow::Pair { left, right } if left == right => seen[left.unwrap()] += 1,
                SplitRow::Pair { left, right } => {
                    for i in [left, right].into_iter().flatten() {
                        seen[i] += 1;
                    }
                }
            }
        }
        assert_eq!(seen, vec![1; all.len()]);
    }

    #[test]
    fn unexpected_orders_still_cover_every_line_once() {
        let kinds = [
            Removed,
            Added,
            Removed,
            Added,
            NoNewlineAtEof,
            NoNewlineAtEof,
            Added,
            NoNewlineAtEof,
        ];
        let index = SplitIndex::from_kinds(kinds);
        assert_eq!(index.total_lines(), kinds.len() as u64);
        let mut seen = vec![0u32; kinds.len()];
        for row in 0..index.total_rows() {
            let cells = index.cells(row).unwrap();
            if cells.left == cells.right {
                seen[cells.left.unwrap() as usize] += 1;
            } else {
                for line in [cells.left, cells.right].into_iter().flatten() {
                    seen[line as usize] += 1;
                }
            }
        }
        assert_eq!(seen, vec![1; kinds.len()]);
    }

    #[test]
    fn rows_and_lines_map_to_each_other_and_ranges_skip_the_gap_inside_a_block() {
        let mut kinds = vec![HunkHeader, Context];
        kinds.extend(std::iter::repeat_n(Removed, 1000));
        kinds.extend(std::iter::repeat_n(Added, 1000));
        kinds.push(Context);
        let index = SplitIndex::from_kinds(kinds.clone());
        assert_eq!(index.total_lines(), 2003);
        assert_eq!(index.total_rows(), 1003);

        let cells = index.cells(502).unwrap();
        assert_eq!(
            (cells.left, cells.right),
            (Some(2 + 500), Some(2 + 1000 + 500))
        );
        assert_eq!(index.row_of_line(2 + 500), Some(502));
        assert_eq!(index.row_of_line(2 + 1000 + 500), Some(502));
        assert_eq!(index.row_of_line(2002), Some(1002));

        assert_eq!(index.line_ranges(500..510), vec![500..510, 1500..1510]);
        assert_eq!(index.line_ranges(0..3), vec![0..3, 1002..1003]);
        assert!(index.line_ranges(2000..2100).is_empty());
    }

    #[test]
    fn the_index_agrees_with_split_rows_for_arbitrary_sequences() {
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let length = (next() % 40) as usize;
            let kinds: Vec<DiffLineKind> = (0..length)
                .map(|_| match next() % 6 {
                    0 => HunkHeader,
                    1 | 2 => Context,
                    3 => Removed,
                    4 => Added,
                    _ => NoNewlineAtEof,
                })
                .collect();
            let index = SplitIndex::from_kinds(kinds.clone());
            assert_eq!(index.total_lines(), kinds.len() as u64);
            let mut rows_seen = 0;
            for row in 0..index.total_rows() {
                let cells = index.cells(row).expect("every row has cells");
                for line in [cells.left, cells.right].into_iter().flatten() {
                    assert_eq!(
                        index.row_of_line(line),
                        Some(row),
                        "kinds={kinds:?} row={row} line={line}"
                    );
                }
                assert!(cells.left.is_some() || cells.right.is_some());
                rows_seen += 1;
            }
            assert_eq!(rows_seen, index.total_rows());
            let ranges = index.line_ranges(0..index.total_rows());
            let covered: u64 = ranges.iter().map(|r| r.end - r.start).sum();
            assert_eq!(covered, kinds.len() as u64, "kinds={kinds:?}");
        }
    }
}
