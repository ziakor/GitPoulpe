use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rune::prefs::Prefs;
use rune::{Env, Runtime};

use crate::perf::percentile;
use crate::state::{DiffMode, State, ViewKind};
use crate::view::view;

fn reference_path() -> Option<PathBuf> {
    std::env::var_os("GITPOULPE_REFERENCE_REPO").map(PathBuf::from)
}

fn rss_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
        / 1024.0
}

struct Samples {
    label: &'static str,
    ms: Vec<f32>,
}

impl Samples {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            ms: Vec::new(),
        }
    }

    fn time<T>(&mut self, work: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = work();
        self.ms.push(start.elapsed().as_secs_f32() * 1000.0);
        out
    }

    fn line(&mut self) -> String {
        self.ms.sort_by(f32::total_cmp);
        let over = |limit: f32| self.ms.iter().filter(|v| **v > limit).count();
        format!(
            "{:<34} n={:<5} p50={:>6.2} p95={:>6.2} p99={:>6.2} max={:>7.2} ms  >8.3ms:{:<4} >16.7ms:{:<4} >33ms:{}",
            self.label,
            self.ms.len(),
            percentile(&self.ms, 0.50),
            percentile(&self.ms, 0.95),
            percentile(&self.ms, 0.99),
            self.ms.last().copied().unwrap_or(0.0),
            over(8.3),
            over(16.7),
            over(33.3),
        )
    }

    fn p99(&self) -> f32 {
        let mut sorted = self.ms.clone();
        sorted.sort_by(f32::total_cmp);
        percentile(&sorted, 0.99)
    }
}

fn frame(rt: &mut Runtime<State>) {
    rt.state.drain();
    rt.update(|s, tx| {
        s.apply_follow(tx);
        s.apply_copy(tx);
    });
    rt.frame(1.0 / 60.0);
}

#[test]
#[ignore = "needs GITPOULPE_REFERENCE_REPO, run with --release -- --ignored --nocapture"]
fn reference_repository_cpu_budget() {
    let Some(path) = reference_path() else {
        eprintln!("GITPOULPE_REFERENCE_REPO is not set");
        return;
    };
    let mut report = vec![format!("reference repository: {}", path.display())];
    let rss_start = rss_mib();

    let started = Instant::now();
    let mut state = State::new(None, Prefs::memory());
    state.setup(Arc::new(|| {}));
    state.open_blocking(path, Duration::from_secs(120));
    let opened = started.elapsed();
    assert_eq!(
        state.entries().len(),
        3200,
        "the reference repository has 3,200 changed entries"
    );
    report.push(format!(
        "open + first status read          {:>8.1} ms  ({} unstaged, {} staged entries)",
        opened.as_secs_f64() * 1000.0,
        state.count(ViewKind::Unstaged),
        state.count(ViewKind::Staged)
    ));

    let first = Instant::now();
    let mut rt = Runtime::new(
        state,
        view,
        (1440.0, 900.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    rt.on_key = Some(crate::keys::on_key);
    frame(&mut rt);
    report.push(format!(
        "first frame (3,051 files listed)  {:>8.1} ms",
        first.elapsed().as_secs_f64() * 1000.0
    ));
    for _ in 0..60 {
        frame(&mut rt);
        std::thread::sleep(Duration::from_millis(5));
    }
    report.push(format!(
        "memory after opening              {:>8.1} MiB resident (start {rss_start:.1})",
        rss_mib()
    ));

    let mut scroll = Samples::new("scroll 3,000 frames, unified");
    for _ in 0..3000 {
        rt.wheel(900.0, 400.0, -50.0);
        scroll.time(|| frame(&mut rt));
    }
    report.push(scroll.line());
    report.push(format!(
        "  reached file {} of {}, {} lines loaded, {:.1} MiB resident",
        rt.state.current.unwrap_or(0) + 1,
        rt.state.files.len(),
        rt.state.loaded_lines(),
        rss_mib()
    ));

    let mut traverse = Samples::new("fast pass over every file");
    let mut peak_lines = 0;
    for _ in 0..12_000 {
        rt.wheel(900.0, 400.0, -900.0);
        traverse.time(|| frame(&mut rt));
        peak_lines = peak_lines.max(rt.state.loaded_lines());
        if rt.state.current == Some(rt.state.files.len() - 1) {
            break;
        }
    }
    report.push(traverse.line());
    report.push(format!(
        "  reached file {} of {}, peak {peak_lines} lines loaded, {:.1} MiB resident",
        rt.state.current.unwrap_or(0) + 1,
        rt.state.files.len(),
        rss_mib()
    ));
    rt.key(rune::KeyEvent {
        key: rune::Key::Home,
        mods: rune::Mods::default(),
    });
    for _ in 0..40 {
        frame(&mut rt);
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut keys = Samples::new("next file (j) x 300");
    for _ in 0..300 {
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("j".into()),
            mods: rune::Mods::default(),
        });
        keys.time(|| frame(&mut rt));
    }
    report.push(keys.line());

    let mut jumps = Samples::new("End then Home x 10");
    for _ in 0..10 {
        for key in [rune::Key::End, rune::Key::Home] {
            rt.key(rune::KeyEvent {
                key,
                mods: rune::Mods::default(),
            });
            jumps.time(|| frame(&mut rt));
            for _ in 0..5 {
                frame(&mut rt);
            }
        }
    }
    report.push(jumps.line());

    let huge = rt
        .state
        .files
        .iter()
        .position(|&i| rt.state.entries()[i].path.ends_with("data/huge.txt"))
        .expect("the huge file is listed");
    rt.update(move |s, tx| {
        s.jump_to(huge);
        crate::keys::align_file(tx, s.entry_at(huge).unwrap().0);
    });
    for _ in 0..30 {
        frame(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut huge_scroll = Samples::new("scroll 50,000-line file, unified");
    for _ in 0..1500 {
        rt.wheel(900.0, 400.0, -120.0);
        huge_scroll.time(|| frame(&mut rt));
    }
    report.push(huge_scroll.line());

    let mut toggle = Samples::new("switch unified to split inside it");
    rt.update(|s, tx| s.toggle_mode(tx));
    toggle.time(|| frame(&mut rt));
    for _ in 0..20 {
        frame(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    report.push(toggle.line());
    let mut split_scroll = Samples::new("scroll 3,000 frames, split");
    for _ in 0..3000 {
        rt.wheel(900.0, 400.0, -50.0);
        split_scroll.time(|| frame(&mut rt));
    }
    report.push(split_scroll.line());
    report.push(format!(
        "final memory                      {:>8.1} MiB resident, {} lines loaded",
        rss_mib(),
        rt.state.loaded_lines()
    ));
    assert_eq!(rt.state.mode, DiffMode::Split);

    let text = report.join("\n");
    eprintln!("\n{text}\n");
    if let Some(out) = std::env::var_os("GITPOULPE_BENCH_OUT") {
        std::fs::write(out, &text).unwrap();
    }

    assert!(
        scroll.p99() < 16.7,
        "regression guard: unified scroll p99 {} ms",
        scroll.p99()
    );
    assert!(
        huge_scroll.p99() < 16.7,
        "regression guard: huge-file scroll p99 {} ms",
        huge_scroll.p99()
    );
    assert!(
        split_scroll.p99() < 16.7,
        "regression guard: split scroll p99 {} ms",
        split_scroll.p99()
    );
    assert!(
        traverse.p99() < 25.0,
        "regression guard: fast pass p99 {} ms (a stress case, not the 60 images per second claim)",
        traverse.p99()
    );
    assert!(
        peak_lines < 2 * crate::state::MAX_LOADED_LINES,
        "regression guard: {peak_lines} lines loaded at once"
    );
    assert!(
        rss_mib() < 1024.0,
        "regression guard: memory {} MiB",
        rss_mib()
    );
}
