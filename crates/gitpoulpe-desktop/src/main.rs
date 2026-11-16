mod engine;
mod state;
mod theme;
mod view;

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

fn demo_light(state: &mut State) {
    state.set_dark(false);
    demo(state);
}

fn main() {
    rune::run(AppSpec::<State> {
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some((720.0, 480.0)),
        setup: Some(|state, waker| state.setup(waker)),
        poll: Some(|state, _, tx| {
            let changed = state.drain();
            state.apply_follow(tx);
            changed
        }),
        scenes: &[("demo", demo), ("demo-light", demo_light)],
        ..AppSpec::new("GitPoulpe", (1280.0, 800.0), State::default, view::view)
    });
}
