//! Rolling timing statistics for the "Debug performance" view.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(12);
const MEMORY_WINDOW: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Waiting for and converting the next camera frame.
    Camera,
    MediaPipe,
    Tongue,
    Processing,
    Osc,
    Interface,
}

impl Stage {
    pub const ALL: [Stage; 6] =
        [Stage::Camera, Stage::MediaPipe, Stage::Tongue, Stage::Processing, Stage::Osc, Stage::Interface];
}

#[derive(Clone, Default)]
pub struct Series {
    samples: VecDeque<(Instant, f32)>,
}

impl Series {
    pub fn push(&mut self, value: f32) {
        let now = Instant::now();
        self.samples.push_back((now, value));
        while self.samples.front().is_some_and(|(t, _)| now.duration_since(*t) > WINDOW) {
            self.samples.pop_front();
        }
    }

    pub fn average(&self, over: Duration) -> Option<f32> {
        let now = Instant::now();
        let (sum, n) = self
            .samples
            .iter()
            .rev()
            .take_while(|(t, _)| now.duration_since(*t) <= over)
            .fold((0.0, 0), |(s, n), (_, v)| (s + v, n + 1));
        (n > 0).then(|| sum / n as f32)
    }

    /// Values from the last 12 s with their age in seconds.
    pub fn history(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let now = Instant::now();
        self.samples.iter().map(move |(t, v)| (now.duration_since(*t).as_secs_f32(), *v))
    }
}

#[derive(Clone, Default)]
pub struct Perf {
    pub stages: BTreeMap<Stage, Series>,
    /// Camera frame arrival to OSC send.
    pub latency: Series,
    pub memory: VecDeque<(Instant, u64)>,
}

impl Perf {
    pub fn record(&mut self, stage: Stage, ms: f32) {
        self.stages.entry(stage).or_default().push(ms);
    }

    pub fn sample_memory(&mut self, bytes: u64) {
        let now = Instant::now();
        self.memory.push_back((now, bytes));
        while self.memory.front().is_some_and(|(t, _)| now.duration_since(*t) > MEMORY_WINDOW) {
            self.memory.pop_front();
        }
    }
}
