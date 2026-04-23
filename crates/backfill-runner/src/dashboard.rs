//! Sticky-at-bottom dashboard for the `run` subcommand.
//!
//! Layout (top to bottom, above the scrolling tracing log area):
//!
//! ```text
//! partition 1/1 (seq 50496000) — stage: indexing
//! parse         avg 12 ms    min/max 2 / 18 ms
//! persist       avg 34 ms    min/max 10 / 80 ms
//! [██████████████░░░░░░]    842 /   1000 ( 84%) elapsed 00:02:15 ETA 00:00:30
//! ```
//!
//! The top three lines (partition, parse, persist) are spinner-style bars
//! with a `{msg}`-only template — effectively labeled strings we
//! `set_message` on. The bottom line is a real progress bar; `pos`/`len`/`%`
//! plus `elapsed` + `ETA` come straight from indicatif template tokens
//! (`{elapsed_precise}` / `{eta_precise}`), so we don't recompute them by
//! hand. `enable_steady_tick` keeps elapsed and ETA refreshing during slow
//! segments (e.g. while `aws s3 sync` runs and no ledgers are advancing).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct Dashboard {
    partition: ProgressBar,
    parse: ProgressBar,
    persist: ProgressBar,
    progress: ProgressBar,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    partition_label: String,
    stage: String,
    parse: Stat,
    persist: Stat,
}

#[derive(Default)]
struct Stat {
    total_ms: u128,
    count: u64,
    min_ms: Option<u128>,
    max_ms: Option<u128>,
}

impl Stat {
    fn observe(&mut self, ms: u128) {
        self.total_ms += ms;
        self.count += 1;
        self.min_ms = Some(self.min_ms.map_or(ms, |m| m.min(ms)));
        self.max_ms = Some(self.max_ms.map_or(ms, |m| m.max(ms)));
    }

    fn format(&self, label: &str) -> String {
        debug_assert!(
            self.count > 0,
            "Stat::format called before observe — caller contract violated",
        );
        let avg = self.total_ms / self.count as u128;
        let min = self.min_ms.expect("min_ms set by observe");
        let max = self.max_ms.expect("max_ms set by observe");
        format!("{label:<14}avg {avg} ms    min/max {min} / {max} ms")
    }
}

impl Dashboard {
    /// `total_range` — all ledgers in the requested `[start, end]` window.
    /// `already_done` — count already in DB (resume state); the visual bar
    /// is pre-bumped so `pos / len / %` reflects the resumed run's real
    /// starting position. `reset_eta()` is called after the pre-bump so the
    /// estimator starts fresh — ETA only reflects this run's actual progress.
    pub fn new(total_range: u64, already_done: u64, mp: &MultiProgress) -> Self {
        // Order matters — `MultiProgress::add` appends, so the first add
        // is the topmost sticky line and the last is bottom-most.

        let partition = make_text_line(mp, "starting…");
        let parse = make_text_line(mp, "parse         —");
        let persist = make_text_line(mp, "persist       —");

        let progress = mp.add(ProgressBar::new(total_range));
        progress.set_style(
            ProgressStyle::with_template(
                "{bar:40} {pos:>6} / {len:>6} ({percent:>3}%) elapsed {elapsed_precise} ETA {eta_precise}",
            )
            .unwrap(),
        );
        progress.inc(already_done);
        // inc(already_done) fires record(already_done, t≈0) in the estimator,
        // producing a spurious rate spike (already_done / ε). Reset so ETA
        // starts from zero and reflects only ledgers processed in this run.
        progress.reset_eta();
        // Keep elapsed + ETA refreshing between `inc(1)` calls (e.g. during
        // a multi-minute `aws s3 sync` where no ledger advances the bar).
        progress.enable_steady_tick(Duration::from_millis(500));

        Self {
            partition,
            parse,
            persist,
            progress,
            state: Mutex::new(State::default()),
        }
    }

    pub fn set_partition(&self, index: usize, total: usize, start_seq: u32) {
        let label = format!("partition {}/{} (seq {start_seq})", index + 1, total);
        let msg = {
            let mut s = self.state.lock().unwrap();
            s.partition_label = label;
            compose_partition(&s)
        };
        self.partition.set_message(msg);
    }

    pub fn set_stage(&self, stage: &str) {
        let msg = {
            let mut s = self.state.lock().unwrap();
            s.stage = stage.to_string();
            compose_partition(&s)
        };
        self.partition.set_message(msg);
    }

    /// Fold one ledger's timings into the dashboard: bump the progress
    /// bar by one and update parse / persist rolling stats. Single call
    /// site in `ingest.rs` — adding a future per-ledger metric is a
    /// one-place change instead of three.
    pub fn record_ledger(&self, parse_ms: u128, persist_ms: u128) {
        self.progress.inc(1);

        let (parse_msg, persist_msg) = {
            let mut s = self.state.lock().unwrap();
            s.parse.observe(parse_ms);
            s.persist.observe(persist_ms);
            (s.parse.format("parse"), s.persist.format("persist"))
        };
        self.parse.set_message(parse_msg);
        self.persist.set_message(persist_msg);
    }

    /// Freeze each bar at its last rendered frame. Intended for the panic
    /// hook path: `finish_and_clear` would erase the dashboard (not what
    /// we want on panic) — `abandon` leaves the frozen frame above the
    /// backtrace.
    pub fn abandon_all(&self) {
        self.partition.abandon();
        self.parse.abandon();
        self.persist.abandon();
        self.progress.abandon();
    }

    pub fn finish_and_clear(&self) {
        self.partition.finish_and_clear();
        self.parse.finish_and_clear();
        self.persist.finish_and_clear();
        self.progress.finish_and_clear();
    }
}

/// Stop ticker threads before the default hook writes the backtrace so the
/// frozen frame stays above it. `abandon`'s own draw is a panic-guarded
/// no-op (indicatif `multi.rs:274`) — the load-bearing effect is killing
/// the ticker, not drawing.
///
/// Lives next to `Dashboard` because it's part of the dashboard's contract:
/// any caller building one is expected to install this hook.
pub fn install_panic_hook(dashboard: Arc<Dashboard>) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dashboard.abandon_all();
        default_hook(info);
    }));
}

fn make_text_line(mp: &MultiProgress, initial: &str) -> ProgressBar {
    let bar = mp.add(ProgressBar::new_spinner());
    bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
    bar.set_message(initial.to_owned());
    bar
}

fn compose_partition(s: &State) -> String {
    match (s.partition_label.is_empty(), s.stage.is_empty()) {
        (true, true) => String::from("starting…"),
        (false, true) => s.partition_label.clone(),
        (true, false) => format!("stage: {}", s.stage),
        (false, false) => format!("{} — stage: {}", s.partition_label, s.stage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_partition_cases() {
        let mut s = State::default();
        assert_eq!(compose_partition(&s), "starting…");

        s.partition_label = "partition 1/3 (seq 50496000)".into();
        assert_eq!(compose_partition(&s), "partition 1/3 (seq 50496000)");

        s.stage = "indexing".into();
        assert_eq!(
            compose_partition(&s),
            "partition 1/3 (seq 50496000) — stage: indexing"
        );

        s.partition_label.clear();
        assert_eq!(compose_partition(&s), "stage: indexing");
    }

    #[test]
    fn stat_observe_tracks_min_max_and_avg() {
        let mut stat = Stat::default();
        stat.observe(10);
        stat.observe(30);
        stat.observe(20);

        assert_eq!(stat.count, 3);
        assert_eq!(stat.total_ms, 60);
        assert_eq!(stat.min_ms, Some(10));
        assert_eq!(stat.max_ms, Some(30));
        assert_eq!(
            stat.format("parse"),
            "parse         avg 20 ms    min/max 10 / 30 ms"
        );
    }
}
