//! OSC output loop. Tracking produces ~30 frames per second; this thread sends at a
//! fixed higher rate and, with interpolation on, blends between the two latest frames
//! so the avatar moves continuously instead of stepping at the camera rate.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::osc::{Binding, OscSender};
use crate::perf::{Perf, Stage};
use crate::system;

const RATE_HZ: f64 = 100.0;
/// Frames older than this are held instead of interpolated toward.
const STALE: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct Frame {
    /// When the camera delivered the frame these values came from.
    pub captured: Instant,
    pub values: Vec<f32>,
}

#[derive(Default)]
pub struct Feed {
    previous: Option<Frame>,
    latest: Option<Frame>,
    sequence: u64,
    pub bindings: Vec<Option<Binding>>,
    pub flags: Vec<String>,
    /// FT output switch: off sends `neutral` and clears the tracking-active flags.
    pub active: bool,
    pub neutral: Vec<f32>,
    pub interpolate: bool,
    pub host: String,
    pub port: u16,
    pub error: Option<String>,
}

impl Feed {
    pub fn push(&mut self, frame: Frame) {
        self.previous = self.latest.replace(frame);
        self.sequence += 1;
    }
}

pub type SharedFeed = Arc<Mutex<Feed>>;

pub fn spawn(perf: Arc<Mutex<Perf>>) -> SharedFeed {
    let feed = SharedFeed::default();
    let shared = feed.clone();
    thread::Builder::new()
        .name("osc-output".into())
        .spawn(move || run(&shared, &perf))
        .expect("spawning OSC output thread");
    feed
}

fn blend(previous: &Frame, latest: &Frame, now: Instant) -> Vec<f32> {
    // Render one frame interval in the past so there is always a frame on each side.
    let interval =
        latest.captured.duration_since(previous.captured).clamp(Duration::from_millis(5), Duration::from_millis(100));
    let render = now.checked_sub(interval).unwrap_or(now);
    let t = if render <= previous.captured {
        0.0
    } else {
        (render.duration_since(previous.captured).as_secs_f32() / interval.as_secs_f32()).min(1.0)
    };
    previous.values.iter().zip(&latest.values).map(|(a, b)| a + (b - a) * t).collect()
}

fn run(feed: &SharedFeed, perf: &Mutex<Perf>) {
    system::high_resolution_timer();
    let mut sender = match OscSender::new() {
        Ok(s) => s,
        Err(e) => {
            feed.lock().error = Some(format!("{e:#}"));
            return;
        }
    };
    let period = Duration::from_secs_f64(1.0 / RATE_HZ);
    let mut next = Instant::now();
    let mut sent_sequence = 0;
    let mut was_active = true;
    loop {
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }

        let (values, bindings, flags, active, first_use) = {
            let f = feed.lock();
            sender.set_target(&f.host, f.port);
            if f.active != was_active {
                sender.invalidate();
                was_active = f.active;
            }
            let fresh = f.sequence != sent_sequence;
            let values = if !f.active {
                Some(f.neutral.clone())
            } else {
                match (&f.previous, &f.latest) {
                    (Some(p), Some(l)) if f.interpolate && l.captured.elapsed() < STALE => {
                        Some(blend(p, l, Instant::now()))
                    }
                    (_, Some(l)) if fresh || !f.interpolate => Some(l.values.clone()),
                    _ => None,
                }
            };
            let first_use = fresh.then(|| f.latest.as_ref().map(|l| l.captured)).flatten();
            sent_sequence = f.sequence;
            (values, f.bindings.clone(), f.flags.clone(), f.active, first_use)
        };
        let Some(values) = values else { continue };

        let started = Instant::now();
        let result = sender.send_frame(&bindings, &values, &flags, active);
        let mut p = perf.lock();
        p.record(Stage::Osc, started.elapsed().as_secs_f32() * 1000.0);
        if let Some(captured) = first_use {
            p.latency.push(captured.elapsed().as_secs_f32() * 1000.0);
        }
        drop(p);
        feed.lock().error = result.err().map(|e| format!("{e:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_moves_between_frames() {
        let t0 = Instant::now();
        let a = Frame { captured: t0, values: vec![0.0] };
        let b = Frame { captured: t0 + Duration::from_millis(40), values: vec![1.0] };
        // Rendering one interval behind: at b.captured + 20 ms we are halfway from a to b.
        let mid = blend(&a, &b, t0 + Duration::from_millis(60));
        assert!((mid[0] - 0.5).abs() < 1e-3);
        assert_eq!(blend(&a, &b, t0 + Duration::from_millis(40))[0], 0.0);
        assert_eq!(blend(&a, &b, t0 + Duration::from_millis(200))[0], 1.0);
    }
}
