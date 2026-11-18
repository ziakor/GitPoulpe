#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub column: usize,
    pub through: Vec<usize>,
    pub closing: Vec<usize>,
    pub opening: Vec<usize>,
    pub linking: Vec<usize>,
    pub continues: bool,
    pub width: usize,
}

#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    lanes: Vec<Option<String>>,
}

impl GraphLayout {
    pub fn new() -> Self {
        Self::default()
    }

    fn free_lane(&mut self) -> usize {
        match self.lanes.iter().position(Option::is_none) {
            Some(lane) => lane,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }

    pub fn push(&mut self, commit: &Commit) -> GraphRow {
        let waiting: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.as_deref() == Some(commit.oid.as_str()))
            .map(|(index, _)| index)
            .collect();
        let column = match waiting.first() {
            Some(first) => *first,
            None => self.free_lane(),
        };
        let closing: Vec<usize> = waiting.iter().copied().skip(1).collect();
        for lane in &closing {
            self.lanes[*lane] = None;
        }
        let through: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(index, lane)| *index != column && lane.is_some())
            .map(|(index, _)| index)
            .collect();
        let mut row = GraphRow {
            column,
            through,
            closing,
            continues: !commit.parents.is_empty(),
            ..GraphRow::default()
        };
        self.lanes[column] = commit.parents.first().cloned();
        for parent in commit.parents.iter().skip(1) {
            match self
                .lanes
                .iter()
                .position(|lane| lane.as_deref() == Some(parent.as_str()))
            {
                Some(existing) => row.linking.push(existing),
                None => {
                    let lane = self.free_lane();
                    self.lanes[lane] = Some(parent.clone());
                    row.opening.push(lane);
                }
            }
        }
        while self.lanes.last().is_some_and(Option::is_none) && self.lanes.len() > column + 1 {
            self.lanes.pop();
        }
        row.width = self
            .lanes
            .len()
            .max(column + 1)
            .max(row.closing.iter().copied().max().map_or(0, |m| m + 1))
            .max(row.through.iter().copied().max().map_or(0, |m| m + 1));
        row
    }

    pub fn waiting(&self) -> usize {
        self.lanes.iter().filter(|lane| lane.is_some()).count()
    }
}

pub fn glyphs(row: &GraphRow, width: usize) -> String {
    let width = width.max(row.width);
    let mut cells: Vec<[char; 2]> = vec![[' ', ' ']; width];
    for lane in &row.through {
        cells[*lane] = ['│', ' '];
    }
    let connect =
        |cells: &mut Vec<[char; 2]>, lane: usize, corner_right: char, corner_left: char| {
            let (from, to) = (row.column.min(lane), row.column.max(lane));
            for cell in cells.iter_mut().take(to).skip(from + 1) {
                let vertical = cell[0] == '│';
                *cell = [if vertical { '┼' } else { '─' }, '─'];
            }
            if lane > row.column {
                cells[row.column][1] = '─';
                cells[lane] = [corner_right, ' '];
            } else {
                cells[lane] = [corner_left, '─'];
            }
        };
    for lane in &row.closing {
        connect(&mut cells, *lane, '╯', '╰');
    }
    for lane in row.opening.iter().chain(&row.linking) {
        connect(&mut cells, *lane, '╮', '╭');
    }
    cells[row.column][0] = '●';
    cells
        .iter()
        .flatten()
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            author: String::new(),
            timestamp: 0,
            subject: String::new(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn a_linear_history_stays_in_one_lane() {
        let mut layout = GraphLayout::new();
        let rows: Vec<GraphRow> = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]
            .iter()
            .map(|c| layout.push(c))
            .collect();
        assert!(
            rows.iter()
                .all(|r| r.column == 0 && r.through.is_empty() && r.width == 1)
        );
        assert_eq!(rows.map_continues(), vec![true, true, false]);
        assert_eq!(layout.waiting(), 0);
    }

    trait Continues {
        fn map_continues(&self) -> Vec<bool>;
    }

    impl Continues for Vec<GraphRow> {
        fn map_continues(&self) -> Vec<bool> {
            self.iter().map(|r| r.continues).collect()
        }
    }

    #[test]
    fn a_merge_opens_a_lane_and_the_fork_point_closes_it() {
        let mut layout = GraphLayout::new();
        let m = layout.push(&commit("m", &["a", "b"]));
        assert_eq!(
            (m.column, m.opening.clone(), m.continues),
            (0, vec![1], true)
        );
        let b = layout.push(&commit("b", &["base"]));
        assert_eq!((b.column, b.through.clone()), (1, vec![0]));
        let a = layout.push(&commit("a", &["base"]));
        assert_eq!((a.column, a.through.clone()), (0, vec![1]));
        let base = layout.push(&commit("base", &[]));
        assert_eq!(base.column, 0);
        assert_eq!(
            base.closing,
            vec![1],
            "the second lane converges into the fork point"
        );
        assert_eq!(layout.waiting(), 0);
    }

    #[test]
    fn an_octopus_merge_links_to_a_parent_already_waited_for() {
        let mut layout = GraphLayout::new();
        let first = layout.push(&commit("x", &["p"]));
        assert_eq!(first.column, 0);
        let merge = layout.push(&commit("m", &["q", "p"]));
        assert_eq!(merge.opening, Vec::<usize>::new());
        assert_eq!(merge.linking, vec![0], "p is already carried by lane 0");
    }

    #[test]
    fn the_glyphs_draw_the_connections_between_lanes() {
        let mut layout = GraphLayout::new();
        let merge = layout.push(&commit("m", &["a", "b"]));
        assert_eq!(glyphs(&merge, merge.width), "●─╮");
        let b = layout.push(&commit("b", &["base"]));
        assert_eq!(glyphs(&b, b.width), "│ ●");
        let a = layout.push(&commit("a", &["base"]));
        assert_eq!(glyphs(&a, a.width), "● │");
        let base = layout.push(&commit("base", &[]));
        assert_eq!(glyphs(&base, base.width), "●─╯");
    }
}
