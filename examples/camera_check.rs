//! Opens a camera and reports the delivered frame size and rate.
//!
//! cargo run --release --example camera_check -- [index] [width] [height] [fps]

use std::time::Instant;

use vrc_openface::camera::{self, CameraSource};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    let args: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let arg = |i: usize, default: u32| args.get(i).copied().unwrap_or(default);
    for entry in camera::list() {
        println!("{}: {}", entry.index, entry.name);
    }
    let mut source = CameraSource::open(arg(0, 0), (arg(1, 640), arg(2, 480)), arg(3, 30))?;
    let first = source.read()?;
    let started = Instant::now();
    let frames = 60;
    let mut convert = 0.0;
    let (mut last, mut repeats) = (Vec::new(), 0);
    for _ in 0..frames {
        let frame = source.read()?;
        convert += source.convert_ms;
        if frame.data == last {
            repeats += 1;
        }
        last = frame.data;
    }
    println!(
        "{repeats} of {frames} frames repeat the one before; mean level {:.1}",
        last.iter().map(|&v| v as f64).sum::<f64>() / last.len() as f64
    );
    println!("convert {:.2} ms per frame", convert / frames as f32);
    println!("{}x{} at {:.1} FPS", first.width, first.height, frames as f64 / started.elapsed().as_secs_f64());
    Ok(())
}
