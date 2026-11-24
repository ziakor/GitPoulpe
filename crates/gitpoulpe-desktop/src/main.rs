mod about;
mod accounts;
mod accounts_view;
mod attachments;
mod attachments_view;
mod branches;
mod branches_view;
mod cloning;
mod cloning_view;
mod conflicts;
mod conflicts_view;
mod deep_link;
mod deep_link_view;
mod engine;
mod external_tools;
mod external_tools_view;
mod favorites;
mod favorites_view;
mod file_edit;
mod file_edit_view;
mod file_history;
mod file_history_view;
mod gitflow_screen;
mod gitflow_screen_view;
mod history_state;
mod history_view;
mod hook_screen;
mod hook_screen_view;
mod ignored;
mod ignored_view;
mod initialize;
mod initialize_view;
mod issues;
mod issues_view;
mod keys;
mod lfs_screen;
mod lfs_screen_view;
mod notices;
mod notices_view;
mod overview;
mod overview_view;
mod palette;
mod palette_view;
mod patch_import;
mod patch_import_view;
mod perf;
mod prefs_migration;
mod profiles;
mod profiles_view;
mod publishing;
mod pull_list;
mod pull_list_view;
mod pull_screen;
mod pull_screen_view;
mod pulling;
mod rebasing;
mod reflogs;
mod reflogs_view;
mod remotes;
mod remotes_view;
mod repo_browser;
mod repo_browser_view;
mod resetting;
mod review_demo;
mod review_files_view;
mod review_side_view;
mod sequencing;
mod shell;
mod shortcuts;
mod shortcuts_view;
mod stashes;
mod stashes_view;
mod state;
mod submodule_screen;
mod submodule_screen_view;
mod tags;
mod tags_view;
mod terminal;
mod terminal_view;
mod theme;
mod undos;
mod undos_view;
mod view;
mod widgets;
mod workspaces;
mod workspaces_view;
mod worktrees;
mod worktrees_view;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::time::Duration;

use rune::AppSpec;

use state::State;

fn demo(state: &mut State) {
    if let Some(path) = std::env::var_os("GITPOULPE_DEMO_REPO") {
        state.open_blocking(PathBuf::from(path), Duration::from_secs(20));
        state.settle(40, Duration::from_secs(20));
        state.drain();
        state.current = (!state.files.is_empty()).then_some(0);
    }
}

fn demo_split(state: &mut State) {
    demo(state);
    state.set_mode(state::DiffMode::Split);
    state.current = (!state.files.is_empty()).then_some(0);
}

fn demo_review(state: &mut State) {
    demo(state);
    review_demo::review_scene(state);
}

fn demo_review_files(state: &mut State) {
    demo(state);
    review_demo::review_files_scene(state);
}

fn demo_device(state: &mut State) {
    demo(state);
    state.accounts.open = true;
    state.accounts.device = Some(gitpoulpe_hosting::DeviceCode {
        device_code: String::new(),
        user_code: "18E5-6DE7".to_owned(),
        verification_uri: "https://github.com/login/device".to_owned(),
        expires_in: 900,
        interval: 5,
    });
}

fn demo_review_light(state: &mut State) {
    state.set_dark(false);
    demo_review(state);
}

fn demo_light(state: &mut State) {
    state.set_dark(false);
    demo(state);
}

fn main() {
    if let Some(text) = about::text_for_args(std::env::args_os()) {
        print!("{text}");
        return;
    }
    rune::App::new(AppSpec::<State> {
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some((720.0, 480.0)),
        on_key: Some(keys::on_key),
        setup: Some(|state, waker| state.setup(waker)),
        poll: Some(|state, t, tx| {
            state.perf_frame(t, tx);
            let mut changed = state.drain();
            state.apply_follow(tx);
            state.apply_copy(tx);
            state.apply_anchor(tx);
            state.apply_reveal(tx);
            changed |= state.apply_terminal(tx);
            changed
        }),
        scenes: &[
            ("demo", demo),
            ("demo-light", demo_light),
            ("demo-split", demo_split),
            ("demo-review", demo_review),
            ("demo-review-light", demo_review_light),
            ("demo-review-files", demo_review_files),
            ("demo-device", demo_device),
        ],
        ..AppSpec::new("GitPoulpe", (1280.0, 800.0), State::default, view::view)
    })
    .on_close(|state, _| state.shutdown())
    .run();
}
