//! [`Progress`]: how far a long job over a run's requests (an export) has got, read from another
//! thread to draw a progress bar, and the flag that cancels it.
//!
//! The job is a known number of steps (each pass over the log, rendering, writing the file). The
//! code running it calls [`Progress::begin_step`] before each one with the number of units it
//! will take (samples, bytes), and the loops of this crate's `*_with` functions report their
//! samples as they go and stop early once the job is cancelled.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// How many units a loop handles between two updates of its [`Progress`]: often enough for a
/// smooth bar, rarely enough that the atomics cost nothing next to the work.
pub const CHUNK: u64 = 16_384;

/// The progress of a job of [`Progress::new`]'s steps, shared between the thread running it and
/// the one showing it. A default `Progress` is one that nobody reads or cancels.
#[derive(Debug, Default)]
pub struct Progress {
    /// How many steps the job has.
    steps: AtomicU32,
    /// How many steps have begun; the last one is in progress.
    begun: AtomicU32,
    /// What the step in progress does, a number of the caller's choice.
    stage: AtomicU32,
    /// The units done of the step in progress.
    done: AtomicU64,
    /// The units of the step in progress.
    total: AtomicU64,
    /// Whether the job has been cancelled.
    cancelled: AtomicBool,
}

impl Progress {
    /// The progress of a job of `steps` steps, none begun.
    pub fn new(steps: u32) -> Self {
        Self {
            steps: AtomicU32::new(steps),
            ..Self::default()
        }
    }

    /// Ends the step in progress, if any, and begins the next one: `stage` says what it does and
    /// `units` how long it is.
    pub fn begin_step(&self, stage: u32, units: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(units, Ordering::Relaxed);
        self.stage.store(stage, Ordering::Relaxed);
        self.begun.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds `units` to the step in progress.
    pub fn advance(&self, units: u64) {
        self.done.fetch_add(units, Ordering::Relaxed);
    }

    /// The `stage` of the step in progress, `0` before the first one.
    pub fn stage(&self) -> u32 {
        self.stage.load(Ordering::Relaxed)
    }

    /// How much of the whole job is done, from 0 to 1: the steps before the one in progress plus
    /// the done part of that one.
    pub fn fraction(&self) -> f32 {
        let steps = self.steps.load(Ordering::Relaxed);
        let begun = self.begun.load(Ordering::Relaxed);
        if steps == 0 || begun == 0 {
            return 0.0;
        }
        let total = self.total.load(Ordering::Relaxed);
        let current = if total == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0)
        };
        let fraction = (f64::from(begun - 1) + current) / f64::from(steps);
        fraction.clamp(0.0, 1.0) as f32
    }

    /// Asks the job to stop. Its loops notice within [`CHUNK`] units.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Reports a loop's units to a [`Progress`] by chunks of [`CHUNK`]. What is left is reported
/// when it is dropped.
pub(crate) struct Ticker<'a> {
    progress: &'a Progress,
    pending: u64,
}

impl<'a> Ticker<'a> {
    /// A ticker reporting to `progress`.
    pub(crate) fn new(progress: &'a Progress) -> Self {
        Self {
            progress,
            pending: 0,
        }
    }

    /// Counts one unit. `false` once the job is cancelled: the loop should stop.
    pub(crate) fn tick(&mut self) -> bool {
        self.pending += 1;
        if self.pending < CHUNK {
            return true;
        }
        self.progress.advance(self.pending);
        self.pending = 0;
        !self.progress.is_cancelled()
    }
}

impl Drop for Ticker<'_> {
    fn drop(&mut self) {
        self.progress.advance(self.pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fraction_adds_the_steps_before_and_the_part_of_the_current_one() {
        let progress = Progress::new(4);
        assert_eq!(progress.fraction(), 0.0);
        progress.begin_step(7, 200);
        progress.advance(100);
        assert_eq!(progress.stage(), 7);
        assert_eq!(progress.fraction(), 0.125);
        progress.begin_step(8, 0);
        assert_eq!(progress.fraction(), 0.25);
        progress.begin_step(9, 10);
        progress.advance(50);
        assert_eq!(progress.fraction(), 0.75);
    }

    #[test]
    fn a_ticker_reports_by_chunks_and_the_rest_when_dropped() {
        let progress = Progress::new(1);
        progress.begin_step(0, CHUNK * 2);
        {
            let mut ticker = Ticker::new(&progress);
            for _ in 0..CHUNK + 5 {
                assert!(ticker.tick());
            }
            assert_eq!(progress.done.load(Ordering::Relaxed), CHUNK);
        }
        assert_eq!(progress.done.load(Ordering::Relaxed), CHUNK + 5);
    }

    #[test]
    fn a_ticker_stops_once_cancelled() {
        let progress = Progress::new(1);
        progress.cancel();
        let mut ticker = Ticker::new(&progress);
        let ticks = (0..CHUNK * 3).take_while(|_| ticker.tick()).count();
        assert_eq!(ticks as u64, CHUNK - 1);
    }
}
