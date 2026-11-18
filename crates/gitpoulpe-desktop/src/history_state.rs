use gitpoulpe_core::{Commit, GraphLayout, GraphRow};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HistoryStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    NoCommits,
    Failed(String),
}

#[derive(Default)]
pub struct HistoryState {
    pub commits: Vec<Commit>,
    pub rows: Vec<GraphRow>,
    layout: GraphLayout,
    pub width: usize,
    pub done: bool,
    pub loading: bool,
    pub request: u64,
    pub status: HistoryStatus,
    pub selected: Option<usize>,
    pub head: Option<String>,
    pub version: u32,
}

impl HistoryState {
    pub fn restart(&mut self, request: u64, head: Option<String>) {
        let version = self.version + 1;
        *self = Self {
            request,
            head,
            loading: true,
            status: HistoryStatus::Loading,
            version,
            ..Self::default()
        };
    }

    pub fn push_page(&mut self, commits: Vec<Commit>, done: bool) {
        for commit in commits {
            let row = self.layout.push(&commit);
            self.width = self.width.max(row.width);
            self.rows.push(row);
            self.commits.push(commit);
        }
        self.done = done;
        self.loading = false;
        self.status = HistoryStatus::Ready;
        self.version += 1;
    }

    pub fn fail(&mut self, no_commits: bool, message: String) {
        self.loading = false;
        self.done = true;
        self.status = if no_commits {
            HistoryStatus::NoCommits
        } else {
            HistoryStatus::Failed(message)
        };
        self.version += 1;
    }

    pub fn needs_more(&self, last_visible: usize) -> bool {
        !self.done && !self.loading && last_visible + 40 >= self.commits.len()
    }
}

#[derive(Default)]
pub struct SearchState {
    pub request: u64,
    pub results: Vec<Commit>,
    pub done: bool,
    pub loading: bool,
    pub status: SearchStatus,
    pub version: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SearchStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    NoResults,
    Failed(String),
}

impl SearchState {
    pub fn restart(&mut self, request: u64) {
        let version = self.version + 1;
        *self = Self {
            request,
            loading: true,
            status: SearchStatus::Loading,
            version,
            ..Self::default()
        };
    }

    pub fn clear(&mut self) {
        let version = self.version + 1;
        let request = self.request + 1;
        *self = Self {
            request,
            version,
            ..Self::default()
        };
    }

    pub fn push_page(&mut self, commits: Vec<Commit>, done: bool) {
        self.results.extend(commits);
        self.done = done;
        self.loading = false;
        self.status = if self.results.is_empty() && done {
            SearchStatus::NoResults
        } else {
            SearchStatus::Ready
        };
        self.version += 1;
    }

    pub fn fail(&mut self, message: String) {
        self.loading = false;
        self.done = true;
        self.status = SearchStatus::Failed(message);
        self.version += 1;
    }

    pub fn needs_more(&self, last_visible: usize) -> bool {
        !self.done && !self.loading && last_visible + 20 >= self.results.len()
    }
}

pub fn format_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_formatted_in_utc_across_leap_years_and_the_epoch() {
        assert_eq!(format_date(0), "1970-01-01 00:00");
        assert_eq!(format_date(951_782_400), "2000-02-29 00:00");
        assert_eq!(format_date(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_date(1_795_000_000), "2026-11-18 11:06");
        assert_eq!(format_date(-86_400), "1969-12-31 00:00");
    }
}
