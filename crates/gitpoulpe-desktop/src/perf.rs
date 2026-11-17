use std::path::PathBuf;

use rune::Tx;

const MAX_INTERVALS: usize = 400_000;
const IDLE_GAP_MS: f64 = 100.0;
const WRITE_EVERY_SECONDS: f64 = 2.0;
const WARMUP_SECONDS: f64 = 1.0;
const SCROLL_SPEED: f32 = 3000.0;
const MAX_EVENTS: usize = 200;
const EVENT_FRAME_MS: f64 = 25.0;
const NOMINAL_FRAME_MS: f64 = 1000.0 / 60.0;
const MIN_COVERAGE: f64 = 0.9;

struct Autoscroll {
    duration: f64,
    speed: f32,
    ready_at: Option<f64>,
    started_at: Option<f64>,
    offset: f32,
    finished: bool,
}

pub struct Perf {
    path: PathBuf,
    last_t: Option<f64>,
    intervals: Vec<f32>,
    idle_gaps: u32,
    last_write: f64,
    first_ready_at: Option<f64>,
    autoscroll: Option<Autoscroll>,
    origin: f64,
    events: Vec<(f64, f64)>,
    window: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub mean: f32,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
    pub over_16_7: usize,
    pub over_25: usize,
    pub over_33: usize,
    pub idle_gaps: u32,
}

pub fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn resident_kib(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

impl Perf {
    pub fn from_env() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("GITPOULPE_PERF_LOG")?);
        let autoscroll = std::env::var("GITPOULPE_AUTOSCROLL")
            .ok()
            .and_then(|seconds| seconds.parse::<f64>().ok())
            .filter(|seconds| *seconds > 0.0)
            .map(|duration| Autoscroll {
                duration,
                speed: SCROLL_SPEED,
                ready_at: None,
                started_at: None,
                offset: 0.0,
                finished: false,
            });
        Some(Self::new(path, autoscroll))
    }

    fn new(path: PathBuf, autoscroll: Option<Autoscroll>) -> Self {
        Self {
            path,
            last_t: None,
            intervals: Vec::new(),
            idle_gaps: 0,
            last_write: 0.0,
            first_ready_at: None,
            autoscroll,
            origin: 0.0,
            events: Vec::new(),
            window: [0.0; 2],
        }
    }

    pub fn summary(&self) -> Summary {
        let mut sorted = self.intervals.clone();
        sorted.sort_by(f32::total_cmp);
        let frames = sorted.len();
        Summary {
            frames,
            mean: if frames == 0 {
                0.0
            } else {
                sorted.iter().sum::<f32>() / frames as f32
            },
            p50: percentile(&sorted, 0.50),
            p95: percentile(&sorted, 0.95),
            p99: percentile(&sorted, 0.99),
            max: sorted.last().copied().unwrap_or(0.0),
            over_16_7: sorted.iter().filter(|v| **v > 16.7).count(),
            over_25: sorted.iter().filter(|v| **v > 25.0).count(),
            over_33: sorted.iter().filter(|v| **v > 33.4).count(),
            idle_gaps: self.idle_gaps,
        }
    }

    fn report(&self) -> String {
        let s = self.summary();
        let scroll = self.autoscroll.as_ref().map_or_else(
            || "null".to_owned(),
            |a| {
                format!(
                    "{{\"duration_s\":{:.1},\"speed_px_s\":{:.0},\"distance_px\":{:.0},\"finished\":{}}}",
                    a.duration, a.speed, a.offset, a.finished
                )
            },
        );
        let coverage = self.autoscroll.as_ref().filter(|a| a.finished).map(|a| {
            let expected = a.duration * 1000.0 / NOMINAL_FRAME_MS;
            s.frames as f64 / expected
        });
        let events = format!(
            "[{}]",
            self.events
                .iter()
                .map(|(at, ms)| format!("[{at:.2},{ms:.1}]"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let validity = match coverage {
            None => "\"unknown\"".to_owned(),
            Some(c) if c >= MIN_COVERAGE => format!("\"valid\",\"coverage\":{c:.2}"),
            Some(c) => format!(
                "\"invalid: the window drew only {:.0}% of the frames a 60 Hz display asks for; keep it visible and focused\",\"coverage\":{c:.2}",
                c * 100.0
            ),
        };
        format!(
            "{{\"validity\":{validity},\"ready_at_s\":{},\"frames\":{},\"interval_ms\":{{\"mean\":{:.2},\"p50\":{:.2},\"p95\":{:.2},\"p99\":{:.2},\"max\":{:.2}}},\"over_16_7ms\":{},\"over_25ms\":{},\"over_33ms\":{},\"idle_gaps\":{},\"window\":{{\"width\":{:.0},\"viewport_height\":{:.0}}},\"autoscroll\":{},\"slow_events\":{},\"rss_kib\":{},\"peak_rss_kib\":{}}}\n",
            self.first_ready_at
                .map_or("null".to_owned(), |t| format!("{t:.2}")),
            s.frames,
            s.mean,
            s.p50,
            s.p95,
            s.p99,
            s.max,
            s.over_16_7,
            s.over_25,
            s.over_33,
            s.idle_gaps,
            self.window[0],
            self.window[1],
            scroll,
            events,
            resident_kib("VmRSS:").unwrap_or(0),
            resident_kib("VmHWM:").unwrap_or(0),
        )
    }

    fn write(&self) {
        let temporary = self.path.with_extension("tmp");
        if std::fs::write(&temporary, self.report()).is_ok() {
            let _ = std::fs::rename(&temporary, &self.path);
        }
    }

    pub fn record_interval(&mut self, t: f64) {
        if let Some(previous) = self.last_t {
            let interval = (t - previous) * 1000.0;
            if (interval > EVENT_FRAME_MS || interval > IDLE_GAP_MS)
                && self.events.len() < MAX_EVENTS
            {
                self.events.push((t - self.origin, interval));
            }
            if interval > IDLE_GAP_MS {
                self.idle_gaps += 1;
            } else if self.intervals.len() < MAX_INTERVALS {
                self.intervals.push(interval as f32);
            }
        }
        self.last_t = Some(t);
    }

    pub fn frame(&mut self, t: f64, ready: bool, window: [f32; 2], tx: &mut Tx) {
        self.window = window;
        if ready && self.first_ready_at.is_none() {
            self.first_ready_at = Some(t);
        }
        let Some(scroll) = self.autoscroll.as_mut() else {
            self.record_interval(t);
            self.maybe_write(t);
            return;
        };
        if scroll.finished {
            return;
        }
        if ready && scroll.ready_at.is_none() {
            scroll.ready_at = Some(t);
        }
        let started = match (scroll.started_at, scroll.ready_at) {
            (Some(start), _) => start,
            (None, Some(ready_at)) if t - ready_at >= WARMUP_SECONDS => {
                scroll.started_at = Some(t);
                self.origin = t;
                self.last_t = None;
                t
            }
            _ => return,
        };
        if t - started >= scroll.duration {
            scroll.finished = true;
            self.write();
            tx.quit();
            return;
        }
        let dt = self.last_t.map_or(0.0, |previous| (t - previous).min(0.05));
        scroll.offset += scroll.speed * dt as f32;
        tx.scroll_to_y("viewport", scroll.offset);
        self.record_interval(t);
    }

    fn maybe_write(&mut self, t: f64) {
        if t - self.last_write >= WRITE_EVERY_SECONDS {
            self.last_write = t;
            self.write();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_pick_the_expected_ranks() {
        let sorted: Vec<f32> = (1..=100).map(|v| v as f32).collect();
        assert_eq!(percentile(&sorted, 0.50), 51.0);
        assert_eq!(percentile(&sorted, 0.95), 95.0);
        assert_eq!(percentile(&sorted, 0.99), 99.0);
        assert_eq!(percentile(&sorted, 1.0), 100.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn intervals_split_frames_from_idle_gaps_and_count_the_slow_ones() {
        let mut perf = Perf::new(PathBuf::from("unused"), None);
        for t in [0.0, 0.016, 0.032, 0.048, 0.068, 0.128, 0.8, 0.816] {
            perf.record_interval(t);
        }
        let summary = perf.summary();
        assert_eq!(
            summary.idle_gaps, 1,
            "the 672 ms pause is idle time, not a frame"
        );
        assert_eq!(summary.frames, 6);
        assert_eq!(
            (summary.over_16_7, summary.over_25, summary.over_33),
            (2, 1, 1)
        );
        assert!((summary.max - 60.0).abs() < 0.01);
        assert!((summary.p50 - 16.0).abs() < 0.01);
        assert_eq!(
            perf.events.len(),
            2,
            "the 60 ms frame and the pause are logged: {:?}",
            perf.events
        );
        assert!((perf.events[0].0 - 0.128).abs() < 1e-9 && (perf.events[0].1 - 60.0).abs() < 0.01);
        assert!(perf.events[1].1 > 600.0);
    }

    #[test]
    fn the_report_is_valid_json_shaped_text_with_every_field() {
        let mut perf = Perf::new(PathBuf::from("unused"), None);
        perf.record_interval(0.0);
        perf.record_interval(0.0167);
        let report = perf.report();
        for field in [
            "validity",
            "ready_at_s",
            "frames",
            "interval_ms",
            "p99",
            "over_16_7ms",
            "idle_gaps",
            "window",
            "autoscroll",
            "rss_kib",
            "peak_rss_kib",
        ] {
            assert!(
                report.contains(&format!("\"{field}\"")),
                "{field} missing in {report}"
            );
        }
        assert!(report.trim_start().starts_with('{') && report.trim_end().ends_with('}'));
    }

    fn finished_scroll() -> Autoscroll {
        Autoscroll {
            duration: 1.0,
            speed: 3000.0,
            ready_at: Some(0.0),
            started_at: Some(0.0),
            offset: 3000.0,
            finished: true,
        }
    }

    #[test]
    fn a_run_that_drew_most_of_the_expected_frames_is_valid_and_a_throttled_one_is_not() {
        let mut healthy = Perf::new(PathBuf::from("unused"), Some(finished_scroll()));
        for frame in 0..58 {
            healthy.record_interval(frame as f64 * 0.01667);
        }
        assert!(
            healthy.report().contains("\"validity\":\"valid\""),
            "{}",
            healthy.report()
        );

        let mut throttled = Perf::new(PathBuf::from("unused"), Some(finished_scroll()));
        for frame in 0..12 {
            throttled.record_interval(frame as f64 * 0.05);
        }
        let report = throttled.report();
        assert!(report.contains("\"validity\":\"invalid"), "{report}");
        assert!(report.contains("keep it visible and focused"));

        let unfinished = Perf::new(PathBuf::from("unused"), None);
        assert!(unfinished.report().contains("\"validity\":\"unknown\""));
    }
}
