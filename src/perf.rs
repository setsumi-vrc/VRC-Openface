//! Rolling timing statistics for the "Debug performance" view.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(12);
const MEMORY_WINDOW: Duration = Duration::from_secs(600);

/// Where a stage runs; also the colour it gets in the latency bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Camera,
    Tracker,
    Core,
    Interface,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Camera, Group::Tracker, Group::Core, Group::Interface];

    pub fn text_key(self) -> &'static str {
        match self {
            Group::Camera => "perf_camera",
            Group::Tracker => "perf_tracker",
            Group::Core => "perf_core",
            Group::Interface => "perf_interface",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Waiting for the next camera frame (Media Foundation's blocking read).
    ReadWait,
    /// Copying the camera's pixels into an RGB frame.
    Convert,
    Mirror,
    MediaPipe,
    Tongue,
    /// Downscaling (and blurring) the camera preview.
    Preview,
    Calibration,
    Mapping,
    /// Handing the frame to the OSC thread and the interface.
    Handoff,
    Osc,
    /// Copying the tracker's snapshot for drawing.
    Snapshot,
    /// Uploading a new preview image to the GPU.
    Upload,
    /// Laying out and painting the three parameter panels.
    Tables,
    /// The whole interface frame.
    Draw,
}

impl Stage {
    pub const ALL: [Stage; 14] = [
        Stage::ReadWait,
        Stage::Convert,
        Stage::Mirror,
        Stage::MediaPipe,
        Stage::Tongue,
        Stage::Preview,
        Stage::Calibration,
        Stage::Mapping,
        Stage::Handoff,
        Stage::Osc,
        Stage::Snapshot,
        Stage::Upload,
        Stage::Tables,
        Stage::Draw,
    ];

    pub fn group(self) -> Group {
        match self {
            Stage::ReadWait | Stage::Convert | Stage::Mirror => Group::Camera,
            Stage::MediaPipe | Stage::Tongue | Stage::Preview => Group::Tracker,
            Stage::Calibration | Stage::Mapping | Stage::Handoff | Stage::Osc => Group::Core,
            Stage::Snapshot | Stage::Upload | Stage::Tables | Stage::Draw => Group::Interface,
        }
    }

    pub fn text_key(self) -> &'static str {
        match self {
            Stage::ReadWait => "st_read_wait",
            Stage::Convert => "st_convert",
            Stage::Mirror => "st_mirror",
            Stage::MediaPipe => "st_mediapipe",
            Stage::Tongue => "st_tongue",
            Stage::Preview => "st_preview",
            Stage::Calibration => "st_calibration",
            Stage::Mapping => "st_mapping",
            Stage::Handoff => "st_handoff",
            Stage::Osc => "st_osc",
            Stage::Snapshot => "st_snapshot",
            Stage::Upload => "st_upload",
            Stage::Tables => "st_tables",
            Stage::Draw => "st_draw",
        }
    }
}

/// Kind of time inside a latency path: work in one of the groups, or waiting for the
/// next part to pick the frame up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Work(Group),
    Waiting,
}

/// From a camera frame to where its result ends up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Path {
    MediaPipeOsc,
    TongueOsc,
    Tables,
    Preview,
}

impl Path {
    pub const ALL: [Path; 4] = [Path::MediaPipeOsc, Path::TongueOsc, Path::Tables, Path::Preview];

    pub fn text_key(self) -> &'static str {
        match self {
            Path::MediaPipeOsc => "path_mediapipe_osc",
            Path::TongueOsc => "path_tongue_osc",
            Path::Tables => "path_tables",
            Path::Preview => "path_preview",
        }
    }
}

/// When a frame passed each point of the tracker. Carried with the frame to the OSC
/// thread and the interface, which add their own part of the latency.
#[derive(Clone, Copy, Debug)]
pub struct FrameTimes {
    pub captured: Instant,
    pub mediapipe: Instant,
    pub tongue: Option<Instant>,
    pub preview: Option<Instant>,
    /// Calibration and mapping done; the frame was handed to the OSC thread.
    pub core: Instant,
    /// Published to the interface.
    pub published: Instant,
}

impl FrameTimes {
    /// End of the tracker's work on this frame (MediaPipe, preview and tongue run one
    /// after the other).
    pub fn tracker_end(&self) -> Instant {
        [Some(self.mediapipe), self.tongue, self.preview].into_iter().flatten().max().unwrap_or(self.mediapipe)
    }
}

fn ms(from: Instant, to: Instant) -> f32 {
    to.saturating_duration_since(from).as_secs_f32() * 1000.0
}

/// Latency breakdown of one frame along a path, as (kind, ms) segments in order.
pub type Breakdown = Vec<(Segment, f32)>;

/// Frame result sent over OSC: `send` is when sending started and how long it took.
pub fn osc_breakdown(times: &FrameTimes, path: Path, send: Instant, send_ms: f32) -> Option<Breakdown> {
    let ready = match path {
        Path::MediaPipeOsc => times.mediapipe,
        Path::TongueOsc => times.tongue?,
        _ => return None,
    };
    let end = times.tracker_end();
    Some(vec![
        (Segment::Work(Group::Tracker), ms(times.captured, ready)),
        // The other tracker steps run before the result can move on.
        (Segment::Waiting, ms(ready, end)),
        (Segment::Work(Group::Core), ms(end, times.core)),
        (Segment::Waiting, ms(times.core, send)),
        (Segment::Work(Group::Core), send_ms),
    ])
}

/// Frame shown in the interface: `frame_started` is when the interface frame that
/// first drew it began, `done` when its part (tables or preview) was drawn.
pub fn interface_breakdown(times: &FrameTimes, frame_started: Instant, done: Instant) -> Breakdown {
    let end = times.tracker_end();
    vec![
        (Segment::Work(Group::Tracker), ms(times.captured, end)),
        (Segment::Work(Group::Core), ms(end, times.published)),
        (Segment::Waiting, ms(times.published, frame_started)),
        (Segment::Work(Group::Interface), ms(frame_started, done)),
    ]
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

/// Latency breakdowns of one path over the last 12 s.
#[derive(Clone, Default)]
pub struct PathSeries {
    samples: VecDeque<(Instant, Breakdown)>,
    /// Totals, for the graph.
    pub total: Series,
}

impl PathSeries {
    pub fn push(&mut self, breakdown: Breakdown) {
        let now = Instant::now();
        self.total.push(breakdown.iter().map(|(_, v)| v).sum());
        self.samples.push_back((now, breakdown));
        while self.samples.front().is_some_and(|(t, _)| now.duration_since(*t) > WINDOW) {
            self.samples.pop_front();
        }
    }

    /// Average of each segment over the last `over`.
    pub fn average(&self, over: Duration) -> Option<Breakdown> {
        let now = Instant::now();
        let recent: Vec<&Breakdown> =
            self.samples.iter().rev().take_while(|(t, _)| now.duration_since(*t) <= over).map(|(_, b)| b).collect();
        let first = recent.first()?;
        let mut sums: Breakdown = first.iter().map(|(kind, _)| (*kind, 0.0)).collect();
        for b in &recent {
            for (sum, (_, v)) in sums.iter_mut().zip(b.iter()) {
                sum.1 += v;
            }
        }
        sums.iter_mut().for_each(|s| s.1 /= recent.len() as f32);
        Some(sums)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MemorySample {
    /// Private bytes (committed memory only this process uses).
    pub private: u64,
    /// Physical memory in use (working set).
    pub working: u64,
}

#[derive(Clone, Default)]
pub struct Perf {
    pub stages: BTreeMap<Stage, Series>,
    pub paths: BTreeMap<Path, PathSeries>,
    pub memory: VecDeque<(Instant, MemorySample)>,
}

impl Perf {
    pub fn record(&mut self, stage: Stage, ms: f32) {
        self.stages.entry(stage).or_default().push(ms);
    }

    pub fn record_path(&mut self, path: Path, breakdown: Breakdown) {
        self.paths.entry(path).or_default().push(breakdown);
    }

    pub fn average(&self, stage: Stage, over: Duration) -> Option<f32> {
        self.stages.get(&stage).and_then(|s| s.average(over))
    }

    pub fn sample_memory(&mut self, sample: MemorySample) {
        let now = Instant::now();
        self.memory.push_back((now, sample));
        while self.memory.front().is_some_and(|(t, _)| now.duration_since(*t) > MEMORY_WINDOW) {
            self.memory.pop_front();
        }
    }
}

/// Least-squares slope of (minutes, MiB) points, in MiB per minute.
pub fn trend_per_minute(points: &[(f32, f32)]) -> Option<f32> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f32;
    let (mx, my) = points.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0 / n, y + p.1 / n));
    let (cov, var) = points.iter().fold((0.0, 0.0), |(c, v), p| (c + (p.0 - mx) * (p.1 - my), v + (p.0 - mx).powi(2)));
    (var > 1e-6).then(|| cov / var)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trend_of_a_line() {
        let points: Vec<(f32, f32)> = (0..10).map(|i| (i as f32, 100.0 + 2.0 * i as f32)).collect();
        assert!((trend_per_minute(&points).unwrap() - 2.0).abs() < 1e-4);
    }

    #[test]
    fn path_average_keeps_segments() {
        let mut series = PathSeries::default();
        series.push(vec![(Segment::Work(Group::Tracker), 2.0), (Segment::Waiting, 4.0)]);
        series.push(vec![(Segment::Work(Group::Tracker), 4.0), (Segment::Waiting, 0.0)]);
        let average = series.average(Duration::from_secs(1)).unwrap();
        assert_eq!(average, vec![(Segment::Work(Group::Tracker), 3.0), (Segment::Waiting, 2.0)]);
    }
}
