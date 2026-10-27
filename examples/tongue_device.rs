//! Compares a tongue model on the CPU and on a DirectML GPU with the same random input.
//!
//! cargo run --release --example tongue_device -- [gpu index] [model]

use std::path::PathBuf;

use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use vrc_openface::face::create_session;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let gpu: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models").join("tongue_model.onnx"));
    let mut cpu = create_session(&path, None)?;
    let mut dml = create_session(&path, Some(gpu))?;
    // The model's input shape, with an open batch set to 1 and open sides to 112.
    let shape: Vec<usize> = cpu
        .inputs()
        .first()
        .and_then(|i| i.dtype().tensor_shape())
        .context("no tensor input")?
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            if d > 0 {
                d as usize
            } else if i == 0 {
                1
            } else {
                112
            }
        })
        .collect();
    let len: usize = shape.iter().product();
    let mut state = 0x9e37_79b9u32;
    let mut worst = 0.0f32;
    for case in 0..6 {
        let input: Vec<f32> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 8) as f32 / (1u32 << 24) as f32
            })
            .collect();
        let run = |session: &mut Session| -> Result<Vec<f32>> {
            let outputs = session.run(ort::inputs![Tensor::from_array((shape.clone(), input.clone()))?])?;
            let (_, values) = outputs[0].try_extract_tensor::<f32>()?;
            Ok(values.to_vec())
        };
        let (a, b) = (run(&mut cpu)?, run(&mut dml)?);
        let diff = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        worst = worst.max(diff);
        println!("case {case}: {} values, max diff {diff:.4}", a.len());
    }
    println!("worst difference {worst:.4} for input {shape:?}");
    Ok(())
}
