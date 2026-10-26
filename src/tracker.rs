//! Capture -> inference -> processing -> OSC loop, running on its own thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Mutex, RwLock};

use crate::camera::CameraSource;
use crate::channels::{self, COUNT};
use crate::config::Settings;
use crate::face::{FaceLandmarker, TongueModel};
use crate::image::RgbFrame;
use crate::osc::{self, Binding, OscSender};
use crate::oscquery::SharedDiscovery;
use crate::processing::Processor;
use crate::unified;

const PREVIEW_MAX_WIDTH: usize = 640;
const READ_FAILURE_LIMIT: u32 = 8;
const RETRY: Duration = Duration::from_secs(1);

pub enum Command {
    CaptureNeutral,
    ResetNeutral,
    CenterGaze,
    ResetGaze,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Stopped,
    Starting,
    Running,
    NoFace,
    CameraError(String),
    ModelError(String),
}

#[derive(Clone)]
pub struct Snapshot {
    pub status: Status,
    pub tracking_fps: f32,
    pub camera_fps: f32,
    pub preview: Option<Arc<RgbFrame>>,
    pub preview_seq: u64,
    pub frame_size: (usize, usize),
    /// Normalised landmarks of the current face, empty when no face is tracked.
    pub landmarks: Vec<[f32; 3]>,
    pub raw: [f32; COUNT],
    pub values: [f32; COUNT],
    pub unified: Vec<f32>,
    pub bindings: Vec<Option<Binding>>,
    pub avatar_id: Option<String>,
    pub avatar_param_count: usize,
    pub vrchat_found: bool,
    pub osc_error: Option<String>,
    pub tongue_error: Option<String>,
    pub capture_progress: Option<f32>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: Status::Starting,
            tracking_fps: 0.0,
            camera_fps: 0.0,
            preview: None,
            preview_seq: 0,
            frame_size: (0, 0),
            landmarks: Vec::new(),
            raw: [0.0; COUNT],
            values: [0.0; COUNT],
            unified: vec![0.0; unified::PARAMS.len()],
            bindings: vec![None; unified::PARAMS.len()],
            avatar_id: None,
            avatar_param_count: 0,
            vrchat_found: false,
            osc_error: None,
            tongue_error: None,
            capture_progress: None,
        }
    }
}

pub struct Shared {
    pub settings: RwLock<Settings>,
    pub snapshot: Mutex<Snapshot>,
}

pub fn spawn(
    shared: Arc<Shared>,
    discovery: SharedDiscovery,
    model_dir: PathBuf,
    repaint: impl Fn() + Send + 'static,
) -> Sender<Command> {
    let (tx, rx) = crossbeam_channel::unbounded();
    thread::Builder::new()
        .name("tracker".into())
        .spawn(move || Tracker::new(shared, discovery, model_dir, rx).run(repaint))
        .expect("spawning tracker thread");
    tx
}

struct FpsCounter {
    started: Instant,
    frames: u32,
    value: f32,
}

impl FpsCounter {
    fn new() -> Self {
        Self { started: Instant::now(), frames: 0, value: 0.0 }
    }

    fn tick(&mut self) -> f32 {
        self.frames += 1;
        let elapsed = self.started.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.value = self.frames as f32 / elapsed;
            self.frames = 0;
            self.started = Instant::now();
        }
        self.value
    }
}

struct Tracker {
    shared: Arc<Shared>,
    discovery: SharedDiscovery,
    model_dir: PathBuf,
    commands: Receiver<Command>,
    camera: Option<CameraSource>,
    camera_key: Option<(u32, (u32, u32), u32)>,
    landmarker: Option<FaceLandmarker>,
    landmarker_gpu: Option<i32>,
    tongue: Option<TongueModel>,
    tongue_error: Option<String>,
    processor: Processor,
    osc: Option<OscSender>,
    bindings: Vec<Option<Binding>>,
    flags: Vec<String>,
    bound_generation: Option<(u64, bool)>,
    avatar_id: Option<String>,
    avatar_param_count: usize,
    vrchat_found: bool,
    started: Instant,
    last_timestamp: u64,
    camera_fps: FpsCounter,
    tracking_fps: FpsCounter,
    preview_seq: u64,
}

impl Tracker {
    fn new(shared: Arc<Shared>, discovery: SharedDiscovery, model_dir: PathBuf, commands: Receiver<Command>) -> Self {
        let osc = OscSender::new().map_err(|e| log::error!("{e:#}")).ok();
        Self {
            shared,
            discovery,
            model_dir,
            commands,
            camera: None,
            camera_key: None,
            landmarker: None,
            landmarker_gpu: None,
            tongue: None,
            tongue_error: None,
            processor: Processor::new(),
            osc,
            bindings: Vec::new(),
            flags: Vec::new(),
            bound_generation: None,
            avatar_id: None,
            avatar_param_count: 0,
            vrchat_found: false,
            started: Instant::now(),
            last_timestamp: 0,
            camera_fps: FpsCounter::new(),
            tracking_fps: FpsCounter::new(),
            preview_seq: 0,
        }
    }

    fn set_status(&self, status: Status) {
        self.shared.snapshot.lock().status = status;
    }

    fn handle_commands(&mut self) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::CaptureNeutral => self.processor.start_neutral_capture(),
                Command::CenterGaze => self.processor.start_gaze_capture(),
                Command::ResetNeutral => self.shared.settings.write().calibration.neutral = None,
                Command::ResetGaze => self.shared.settings.write().calibration.gaze_center = None,
            }
        }
    }

    fn run(mut self, repaint: impl Fn()) {
        let mut logged = None;
        let mut last_fps_log = Instant::now();
        loop {
            let (status, tracking_fps, camera_fps) = {
                let s = self.shared.snapshot.lock();
                (s.status.clone(), s.tracking_fps, s.camera_fps)
            };
            if logged.as_ref() != Some(&status) {
                log::info!("status: {status:?}");
                logged = Some(status);
            }
            if last_fps_log.elapsed() >= Duration::from_secs(10) {
                log::debug!("tracking {tracking_fps:.1} FPS, camera {camera_fps:.1} FPS");
                last_fps_log = Instant::now();
            }
            self.handle_commands();
            let settings = self.shared.settings.read().clone();
            if !settings.tracking_enabled {
                self.camera = None;
                self.camera_key = None;
                self.set_status(Status::Stopped);
                repaint();
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            if let Err(status) = self.step(&settings) {
                self.set_status(status);
                repaint();
                thread::sleep(RETRY);
                continue;
            }
            repaint();
        }
    }

    fn ensure_camera(&mut self, settings: &Settings) -> Result<(), Status> {
        let key = (settings.camera_index, settings.resolution, settings.camera_fps);
        if self.camera_key != Some(key) {
            self.camera = None;
            self.camera_key = Some(key);
        }
        if self.camera.is_none() {
            self.set_status(Status::Starting);
            let camera = CameraSource::open(key.0, key.1, key.2).map_err(|e| Status::CameraError(format!("{e:#}")))?;
            self.camera = Some(camera);
            self.camera_fps = FpsCounter::new();
            self.tracking_fps = FpsCounter::new();
        }
        Ok(())
    }

    fn ensure_models(&mut self, settings: &Settings) -> Result<(), Status> {
        if self.landmarker.is_none() || self.landmarker_gpu != settings.gpu_device {
            self.landmarker = None;
            self.tongue = None;
            self.tongue_error = None;
            let landmarker = FaceLandmarker::load(&self.model_dir, settings.gpu_device)
                .map_err(|e| Status::ModelError(format!("{e:#}")))?;
            self.landmarker = Some(landmarker);
            self.landmarker_gpu = settings.gpu_device;
        }
        if !settings.tongue {
            self.tongue = None;
            self.tongue_error = None;
        } else if self.tongue.is_none() && self.tongue_error.is_none() {
            let path = self.model_dir.join("tongue_model.onnx");
            if path.is_file() {
                match TongueModel::load(&path, settings.gpu_device) {
                    Ok(model) => self.tongue = Some(model),
                    Err(e) => self.tongue_error = Some(format!("{e:#}")),
                }
            } else {
                self.tongue_error = Some(format!("no tongue model at {}", path.display()));
            }
        }
        Ok(())
    }

    fn refresh_bindings(&mut self, settings: &Settings) {
        let discovery = self.discovery.lock();
        let key = (discovery.generation, settings.send_without_avatar);
        if self.bound_generation == Some(key) {
            return;
        }
        self.bound_generation = Some(key);
        self.vrchat_found = discovery.endpoint.is_some();
        match &discovery.avatar {
            Some(avatar) => {
                self.bindings = osc::bind(avatar);
                self.flags = osc::tracking_flags(avatar);
                self.avatar_id = avatar.id.clone();
                self.avatar_param_count = avatar.params.len();
            }
            None => {
                self.bindings = if settings.send_without_avatar {
                    osc::default_bindings()
                } else {
                    vec![None; unified::PARAMS.len()]
                };
                self.flags.clear();
                self.avatar_id = None;
                self.avatar_param_count = 0;
            }
        }
    }

    fn step(&mut self, settings: &Settings) -> Result<(), Status> {
        self.ensure_camera(settings)?;
        self.ensure_models(settings)?;

        let camera = self.camera.as_mut().expect("camera opened above");
        let mut failures = 0;
        let mut frame = loop {
            match camera.read() {
                Ok(frame) => break frame,
                Err(e) => {
                    failures += 1;
                    if failures >= READ_FAILURE_LIMIT {
                        self.camera = None;
                        return Err(Status::CameraError(format!("no frames: {e:#}")));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
        };
        let camera_fps = self.camera_fps.tick();
        if settings.flip_camera {
            frame.flip_horizontal();
        }

        let timestamp = (self.started.elapsed().as_millis() as u64).max(self.last_timestamp + 1);
        self.last_timestamp = timestamp;
        let landmarker = self.landmarker.as_mut().expect("models loaded above");
        let face = landmarker.detect(&frame, timestamp).map_err(|e| Status::ModelError(format!("{e:#}")))?;

        let preview = settings.preview.then(|| {
            self.preview_seq += 1;
            Arc::new(frame.downscale_to_width(PREVIEW_MAX_WIDTH))
        });

        let Some(face) = face else {
            if let Some(tongue) = &mut self.tongue {
                tongue.reset();
            }
            let mut snapshot = self.shared.snapshot.lock();
            snapshot.status = Status::NoFace;
            snapshot.camera_fps = camera_fps;
            snapshot.frame_size = (frame.width, frame.height);
            snapshot.landmarks.clear();
            if preview.is_some() {
                snapshot.preview = preview;
                snapshot.preview_seq = self.preview_seq;
            }
            return Ok(());
        };
        let tracking_fps = self.tracking_fps.tick();

        let mut raw = [0.0f32; COUNT];
        raw[..face.blendshapes.len()].copy_from_slice(&face.blendshapes);
        if settings.tongue
            && let Some(tongue) = &mut self.tongue
        {
            match tongue.update(&frame, &face.pixel_landmarks) {
                Ok(values) => raw[channels::MEDIAPIPE.len()..].copy_from_slice(&values),
                Err(e) => log::debug!("tongue: {e:#}"),
            }
        }

        let processed = {
            let mut s = self.shared.settings.write();
            self.processor.process(&raw, timestamp, settings.smoothing, settings.symmetric, &mut s.calibration)
        };
        let unified = unified::evaluate_all(&processed.values, processed.gaze);

        self.refresh_bindings(settings);
        let mut osc_error = None;
        if let Some(osc) = &mut self.osc {
            osc.set_target(&settings.osc_host, settings.osc_port);
            if let Err(e) = osc.send_frame(&self.bindings, &unified, &self.flags) {
                osc_error = Some(format!("{e:#}"));
            }
        }

        let mut snapshot = self.shared.snapshot.lock();
        snapshot.status = Status::Running;
        snapshot.tracking_fps = tracking_fps;
        snapshot.frame_size = (frame.width, frame.height);
        snapshot.camera_fps = camera_fps;
        if preview.is_some() {
            snapshot.preview = preview;
            snapshot.preview_seq = self.preview_seq;
        }
        snapshot.landmarks = face.landmarks;
        snapshot.raw = raw;
        snapshot.values = processed.values;
        snapshot.unified = unified;
        snapshot.bindings.clone_from(&self.bindings);
        snapshot.avatar_id.clone_from(&self.avatar_id);
        snapshot.avatar_param_count = self.avatar_param_count;
        snapshot.vrchat_found = self.vrchat_found;
        snapshot.osc_error = osc_error;
        snapshot.tongue_error.clone_from(&self.tongue_error);
        snapshot.capture_progress = self.processor.capture_progress();
        Ok(())
    }
}
