use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeKind {
    Review,
    Mention,
    Checks,
    Activity,
    Conflicts,
    System,
}

impl NoticeKind {
    pub const ALL: [Self; 6] = [
        Self::Review,
        Self::Mention,
        Self::Checks,
        Self::Activity,
        Self::Conflicts,
        Self::System,
    ];

    pub fn caption(self) -> &'static str {
        match self {
            Self::Review => "Demandes de revue",
            Self::Mention => "Mentions",
            Self::Checks => "Vérifications (CI)",
            Self::Activity => "Autre activité",
            Self::Conflicts => "Conflits à résoudre",
            Self::System => "Alertes de l'application",
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, Self::Conflicts | Self::System)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub connection: Option<String>,
    pub repo_path: Option<String>,
    pub kind: NoticeKind,
    pub title: String,
    pub repo: Option<String>,
    pub number: Option<u64>,
    pub at: String,
    pub count: u32,
    pub read: bool,
}

impl Notice {
    pub fn group_key(&self) -> String {
        format!(
            "{}|{}|{:?}|{}|{}",
            self.connection.as_deref().unwrap_or("-"),
            self.repo_path.as_deref().unwrap_or("-"),
            self.kind,
            self.repo.as_deref().unwrap_or("-").to_lowercase(),
            self.number
                .map_or_else(|| self.title.clone(), |n| n.to_string()),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoticePrefs {
    pub enabled: bool,
    pub muted_kinds: BTreeSet<NoticeKind>,
    pub muted_connections: BTreeSet<String>,
}

impl Default for NoticePrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            muted_kinds: BTreeSet::new(),
            muted_connections: BTreeSet::new(),
        }
    }
}

impl NoticePrefs {
    pub fn allows(&self, kind: NoticeKind, connection: Option<&str>) -> bool {
        self.enabled
            && !self.muted_kinds.contains(&kind)
            && connection.is_none_or(|c| !self.muted_connections.contains(c))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoticeBook {
    pub items: Vec<Notice>,
    pub prefs: NoticePrefs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Added,
    Grouped,
    Duplicate,
    Muted,
}

const KEEP: usize = 200;

impl NoticeBook {
    pub fn push(&mut self, mut notice: Notice) -> Pushed {
        if !self.prefs.allows(notice.kind, notice.connection.as_deref()) {
            return Pushed::Muted;
        }
        let key = notice.group_key();
        if let Some(existing) = self.items.iter_mut().find(|n| n.group_key() == key) {
            if existing.id == notice.id && existing.at == notice.at {
                return Pushed::Duplicate;
            }
            existing.count += 1;
            existing.at = notice.at;
            existing.id = notice.id;
            existing.title = notice.title;
            existing.read = false;
            return Pushed::Grouped;
        }
        notice.count = notice.count.max(1);
        self.items.insert(0, notice);
        self.items.truncate(KEEP);
        Pushed::Added
    }

    pub fn unread(&self) -> usize {
        self.items
            .iter()
            .filter(|n| !n.read && self.prefs.allows(n.kind, n.connection.as_deref()))
            .count()
    }

    pub fn mark_read(&mut self, id: &str) {
        if let Some(notice) = self.items.iter_mut().find(|n| n.id == id) {
            notice.read = true;
        }
    }

    pub fn mark_all_read(&mut self) {
        for notice in &mut self.items {
            notice.read = true;
        }
    }

    pub fn dismiss(&mut self, id: &str) {
        self.items.retain(|n| n.id != id);
    }

    pub fn forget_connection(&mut self, connection: &str) {
        self.items
            .retain(|n| n.connection.as_deref() != Some(connection));
        self.prefs.muted_connections.remove(connection);
    }

    pub fn visible(&self) -> impl Iterator<Item = &Notice> {
        self.items
            .iter()
            .filter(|n| self.prefs.allows(n.kind, n.connection.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(
        id: &str,
        kind: NoticeKind,
        connection: Option<&str>,
        number: Option<u64>,
        at: &str,
    ) -> Notice {
        Notice {
            id: id.to_owned(),
            connection: connection.map(str::to_owned),
            repo_path: None,
            kind,
            title: format!("title {id}"),
            repo: Some("acme/tool".to_owned()),
            number,
            at: at.to_owned(),
            count: 1,
            read: false,
        }
    }

    #[test]
    fn identical_events_are_ignored_and_a_burst_on_one_subject_is_grouped() {
        let mut book = NoticeBook::default();
        let first = notice("1", NoticeKind::Review, Some("gh/a"), Some(7), "t1");
        assert_eq!(book.push(first.clone()), Pushed::Added);
        assert_eq!(book.push(first), Pushed::Duplicate);
        assert_eq!(book.items[0].count, 1);
        book.mark_read("1");
        assert_eq!(book.unread(), 0);
        for (i, at) in ["t2", "t3", "t4"].into_iter().enumerate() {
            let burst = notice(
                &format!("b{i}"),
                NoticeKind::Review,
                Some("gh/a"),
                Some(7),
                at,
            );
            assert_eq!(book.push(burst), Pushed::Grouped);
        }
        assert_eq!(book.items.len(), 1);
        assert_eq!(
            (
                book.items[0].count,
                book.items[0].at.as_str(),
                book.unread()
            ),
            (4, "t4", 1)
        );
    }

    #[test]
    fn the_same_subject_on_another_connection_or_kind_is_never_merged() {
        let mut book = NoticeBook::default();
        book.push(notice("1", NoticeKind::Review, Some("gh/a"), Some(7), "t"));
        book.push(notice("2", NoticeKind::Review, Some("gh/b"), Some(7), "t"));
        book.push(notice("3", NoticeKind::Checks, Some("gh/a"), Some(7), "t"));
        book.push(notice("4", NoticeKind::Review, Some("gh/a"), Some(8), "t"));
        assert_eq!(book.items.len(), 4);
    }

    #[test]
    fn preferences_mute_kinds_connections_and_everything_and_hide_what_is_muted() {
        let mut book = NoticeBook::default();
        book.prefs.muted_kinds.insert(NoticeKind::Mention);
        assert_eq!(
            book.push(notice("1", NoticeKind::Mention, Some("gh/a"), Some(1), "t")),
            Pushed::Muted
        );
        book.push(notice("2", NoticeKind::Review, Some("gh/a"), Some(2), "t"));
        book.push(notice("3", NoticeKind::Review, Some("gh/b"), Some(3), "t"));
        book.prefs.muted_connections.insert("gh/b".to_owned());
        assert_eq!(book.visible().count(), 1);
        assert_eq!(book.unread(), 1);
        book.prefs.enabled = false;
        assert_eq!(
            book.push(notice("4", NoticeKind::Review, None, Some(4), "t")),
            Pushed::Muted
        );
        assert_eq!(book.unread(), 0);
    }

    #[test]
    fn forgetting_a_connection_removes_only_its_notices_and_the_old_format_loads() {
        let mut book = NoticeBook::default();
        book.push(notice("1", NoticeKind::Review, Some("gh/a"), Some(1), "t"));
        book.push(notice("2", NoticeKind::Review, Some("gh/b"), Some(1), "t"));
        book.push(notice("3", NoticeKind::Conflicts, None, None, "t"));
        book.prefs.muted_connections.insert("gh/a".to_owned());
        book.forget_connection("gh/a");
        let ids: Vec<&str> = book.items.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["3", "2"]);
        assert!(book.prefs.muted_connections.is_empty());
        let old: NoticeBook = serde_json::from_str("{}").unwrap();
        assert!(old.prefs.enabled && old.items.is_empty());
        let text = serde_json::to_string(&book).unwrap();
        assert_eq!(serde_json::from_str::<NoticeBook>(&text).unwrap(), book);
        book.dismiss("2");
        book.mark_all_read();
        assert_eq!(book.items.len(), 1);
        assert_eq!(book.unread(), 0);
    }

    #[test]
    fn the_inbox_is_bounded() {
        let mut book = NoticeBook::default();
        for n in 0..250u64 {
            book.push(notice(
                &n.to_string(),
                NoticeKind::Activity,
                Some("gh/a"),
                Some(n),
                "t",
            ));
        }
        assert_eq!(book.items.len(), 200);
        assert_eq!(book.items[0].id, "249");
    }
}
