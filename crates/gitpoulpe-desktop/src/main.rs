mod branches;
mod branches_view;
mod cloning;
mod cloning_view;
mod conflicts;
mod conflicts_view;
mod engine;
mod history_state;
mod history_view;
mod hook_screen;
mod hook_screen_view;
mod ignored;
mod ignored_view;
mod initialize;
mod initialize_view;
mod keys;
mod perf;
mod profiles;
mod profiles_view;
mod publishing;
mod pulling;
mod rebasing;
mod reflogs;
mod reflogs_view;
mod remotes;
mod remotes_view;
mod resetting;
mod sequencing;
mod shell;
mod stashes;
mod stashes_view;
mod state;
mod tags;
mod tags_view;
mod theme;
mod undos;
mod undos_view;
mod view;
mod widgets;
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

fn demo_light(state: &mut State) {
    state.set_dark(false);
    demo(state);
}

fn main() {
    rune::App::new(AppSpec::<State> {
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some((720.0, 480.0)),
        on_key: Some(keys::on_key),
        setup: Some(|state, waker| state.setup(waker)),
        poll: Some(|state, t, tx| {
            state.perf_frame(t, tx);
            let changed = state.drain();
            state.apply_follow(tx);
            state.apply_copy(tx);
            state.apply_anchor(tx);
            state.apply_reveal(tx);
            changed
        }),
        scenes: &[
            ("demo", demo),
            ("demo-light", demo_light),
            ("demo-split", demo_split),
        ],
        ..AppSpec::new("GitPoulpe", (1280.0, 800.0), State::default, view::view)
    })
    .on_close(|state, _| state.shutdown())
    .run();
}
