use crate::state::State;

pub const NAV_AUTO_MIN: f32 = 1100.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Changes,
    History,
    Pulls,
    Issues,
    Notices,
    Branches,
    Tags,
    Remotes,
    Stashes,
    Worktrees,
    Conflicts,
    Reflog,
    Undo,
    Ignored,
    Hooks,
    Lfs,
    Submodules,
    Gitflow,
    Patches,
    Accounts,
    Profiles,
    Tools,
    Shortcuts,
    Favorites,
    Workspaces,
    Overview,
    Other,
}

impl Place {
    pub fn advanced(self) -> bool {
        matches!(
            self,
            Self::Reflog
                | Self::Undo
                | Self::Ignored
                | Self::Hooks
                | Self::Lfs
                | Self::Submodules
                | Self::Gitflow
                | Self::Patches
        )
    }

    pub fn repositories(self) -> bool {
        matches!(self, Self::Favorites | Self::Workspaces | Self::Overview)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavState {
    pub shown: Option<bool>,
    pub more: bool,
    pub repositories: bool,
}

impl State {
    pub fn place(&self) -> Place {
        let flags = [
            (self.links.open, Place::Other),
            (self.patches.open, Place::Patches),
            (self.issues.open, Place::Issues),
            (self.notices.open, Place::Notices),
            (self.overview.open, Place::Overview),
            (self.favorites.open, Place::Favorites),
            (self.workspaces.open, Place::Workspaces),
            (self.shortcuts.open, Place::Shortcuts),
            (self.tools.open, Place::Tools),
            (self.clone.open, Place::Other),
            (self.init.open, Place::Other),
            (self.accounts.open, Place::Accounts),
            (self.profiles_open, Place::Profiles),
            (self.history_open, Place::History),
            (self.ignored.open, Place::Ignored),
            (self.branches.open, Place::Branches),
            (self.tags.open, Place::Tags),
            (self.remotes.open, Place::Remotes),
            (self.conflicts.open, Place::Conflicts),
            (self.stashes.open, Place::Stashes),
            (self.reflog.open, Place::Reflog),
            (self.file_history.open, Place::Other),
            (self.file_edit.open, Place::Other),
            (self.terminal.open, Place::Worktrees),
            (self.undo_screen.open, Place::Undo),
            (self.worktrees.open, Place::Worktrees),
            (self.hooks.open, Place::Hooks),
            (self.gitflow.open, Place::Gitflow),
            (self.pulls.open, Place::Pulls),
            (self.submodules.open, Place::Submodules),
            (self.lfs.open, Place::Lfs),
        ];
        flags
            .into_iter()
            .find_map(|(open, place)| open.then_some(place))
            .unwrap_or(Place::Changes)
    }

    pub fn close_screens(&mut self) {
        self.close_deep_link();
        self.close_patch_import();
        self.close_issues();
        self.close_notices();
        self.close_overview();
        self.close_favorites();
        self.close_workspaces();
        self.close_shortcuts();
        self.close_tools();
        self.close_clone();
        self.init.open = false;
        self.close_accounts();
        self.close_profiles();
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.close_file_history();
        self.close_file_edit();
        self.close_terminal();
        self.close_undo();
        self.close_worktrees();
        self.close_hooks();
        self.close_gitflow();
        self.close_pulls();
        self.close_submodules();
        self.close_lfs();
    }

    pub fn go_changes(&mut self) {
        self.close_screens();
    }

    pub fn nav_shown(&self) -> bool {
        self.nav
            .shown
            .unwrap_or(self.window_width <= 0.0 || self.window_width >= NAV_AUTO_MIN)
    }

    pub fn toggle_nav(&mut self) {
        self.nav.shown = Some(!self.nav_shown());
    }

    pub fn toggle_nav_more(&mut self) {
        self.nav.more = !self.nav.more;
    }

    pub fn toggle_nav_repositories(&mut self) {
        self.nav.repositories = !self.nav.repositories;
    }
}
