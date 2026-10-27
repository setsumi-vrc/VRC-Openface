//! Capture -> inference -> processing loop on its own thread. Results go to the OSC
//! output thread (`output.rs`) and to the UI as a `Snapshot`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Mutex, RwLock};

use crate::avatar::{self, SharedReceiver};
use crate::camera::CameraSource;
use crate::channels::{self, COUNT};
use crate::config::{self, Device, Settings};
use crate::face::{self, FaceLandmarker, TongueModel};
use crate::image::RgbFrame;
use crate::mapping::{Mapper, OUTPUTS};
use crate::osc::{self, AvatarInfo, Binding};
use crate::oscquery::SharedDiscovery;
use crate::output::{Frame, SharedFeed};
use crate::perf::{FrameTimes, Perf, Stage};
use crate::processing::{CalibrationStore, CaptureOutcome, Processor};
use crate::system::{self, GpuAdapter};

const PREVIEW_MAX_WIDTH: usize = 640;
const READ_FAILURE_LIMIT: u32 = 8;
const RETRY: Duration = Duration::from_secs(1);
pub const TONGUE_FILE: &str = "tongue_model.onnx";

pub enum Command {
    CaptureNeutral,
    UndoNeutral,
    CenterGaze,
    ResetGaze,
    /// Re-read models after an import or a device change.
    ReloadModels,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Starting,
    Running,
    NoFace,
    CameraError(String),
    ModelError(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TongueStatus {
    Off,
    Missing,
    Ready,
    Error(String),
}

#[derive(Clone)]
pub struct Snapshot {
    /// Timestamps of the frame behind these values, and a counter that changes with it.
    pub times: Option<FrameTimes>,
    pub frame_seq: u64,
    pub status: Status,
    pub tracking_fps: f32,
    pub camera_fps: f32,
    pub preview: Option<Arc<RgbFrame>>,
    pub preview_seq: u64,
    pub frame_size: (usize, usize),
    /// Normalised landmarks of the current face, empty when no face is tracked.
    pub landmarks: Vec<[f32; 3]>,
    /// Smoothed inputs before calibration.
    pub input: [f32; COUNT],
    pub values: [f32; COUNT],
    pub outputs: Vec<f32>,
    pub bindings: Vec<Option<Binding>>,
    pub avatar_id: Option<String>,
    pub avatar_name: Option<String>,
    /// Face tracking parameter names (after `v2/`) the avatar declares.
    pub avatar_ft_params: Vec<String>,
    pub vrchat_found: bool,
    /// The avatar comes from a connected VRChat, not from the last session.
    pub avatar_live: bool,
    pub tongue: TongueStatus,
    pub face_device: String,
    pub tongue_device: String,
    pub capture_progress: Option<f32>,
    pub message: Option<(&'static str, Instant)>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            times: None,
            frame_seq: 0,
            status: Status::Starting,
            tracking_fps: 0.0,
            camera_fps: 0.0,
            preview: None,
            preview_seq: 0,
            frame_size: (0, 0),
            landmarks: Vec::new(),
            input: [0.0; COUNT],
            values: [0.0; COUNT],
            outputs: vec![0.0; OUTPUTS.len()],
            bindings: vec![None; OUTPUTS.len()],
            avatar_id: None,
            avatar_name: None,
            avatar_ft_params: Vec::new(),
            vrchat_found: false,
            avatar_live: false,
            tongue: TongueStatus::Off,
            face_device: String::new(),
            tongue_device: String::new(),
            capture_progress: None,
            message: None,
        }
    }
}

pub struct Shared {
    pub settings: RwLock<Settings>,
    pub calibrations: Mutex<CalibrationStore>,
    pub calibrations_dirty: AtomicBool,
    pub snapshot: Mutex<Snapshot>,
    pub perf: Arc<Mutex<Perf>>,
    /// False while the interface sleeps; the tracker then stops requesting repaints.
    pub ui_awake: AtomicBool,
    pub receive_port: Arc<AtomicU16>,
    pub gpus: Vec<GpuAdapter>,
    /// The preview is shown unblurred until this time (set by hovering the preview).
    pub unblurred_until: Mutex<Option<Instant>>,
}

impl Shared {
    pub fn new(settings: Settings) -> Self {
        let port = settings.receive_port;
        Self {
            settings: RwLock::new(settings),
            calibrations: Mutex::new(config::load_calibrations()),
            calibrations_dirty: AtomicBool::new(false),
            snapshot: Mutex::new(Snapshot::default()),
            perf: Arc::default(),
            ui_awake: AtomicBool::new(true),
            receive_port: Arc::new(AtomicU16::new(port)),
            gpus: system::gpu_adapters(),
            unblurred_until: Mutex::new(None),
        }
    }
}

pub struct Sources {
    pub discovery: SharedDiscovery,
    pub receiver: SharedReceiver,
    pub feed: SharedFeed,
}

pub fn spawn(
    shared: Arc<Shared>,
    sources: Sources,
    model_dir: PathBuf,
    repaint: impl Fn() + Send + 'static,
) -> Sender<Command> {
    let (tx, rx) = crossbeam_channel::unbounded();
    thread::Builder::new()
        .name("tracker".into())
        .spawn(move || {
            let awake = shared.clone();
            let repaint = move || {
                if awake.ui_awake.load(Ordering::Relaxed) {
                    repaint();
                }
            };
            Tracker::new(shared, sources, model_dir, rx).run(repaint)
        })
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

fn ms(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1000.0
}

/// Auto picks the first hardware GPU; DirectML device ids are DXGI adapter indices.
fn resolve(device: Device, gpus: &[GpuAdapter]) -> Option<i32> {
    match device {
        Device::Cpu => None,
        Device::Gpu(i) => Some(i as i32),
        Device::Auto if cfg!(feature = "directml") => gpus.first().map(|g| g.index as i32),
        Device::Auto => None,
    }
}

fn device_label(gpu: Option<i32>, gpus: &[GpuAdapter]) -> String {
    match gpu {
        None => "CPU".into(),
        Some(i) => match gpus.iter().find(|g| g.index as i32 == i) {
            Some(g) => format!("GPU {i}: {}", g.name),
            None => format!("GPU {i}"),
        },
    }
}

/// Imported tongue model first, then the models folder. Without either, a build that
/// compiled one in uses that.
pub fn tongue_path(model_dir: &Path) -> Option<PathBuf> {
    [config::user_model_dir().join(TONGUE_FILE), model_dir.join(TONGUE_FILE)].into_iter().find(|p| p.is_file())
}

struct Avatar {
    key: Option<(u64, u64, Option<String>, bool, bool)>,
    info: AvatarInfo,
    bindings: Vec<Option<Binding>>,
    flags: Vec<String>,
    ft_params: Vec<String>,
    vrchat_found: bool,
    live: bool,
}

struct Tracker {
    shared: Arc<Shared>,
    sources: Sources,
    model_dir: PathBuf,
    commands: Receiver<Command>,
    camera: Option<CameraSource>,
    camera_key: Option<(u32, (u32, u32), u32)>,
    landmarker: Option<FaceLandmarker>,
    face_gpu: Option<Option<i32>>,
    tongue: Option<TongueModel>,
    tongue_gpu: Option<Option<i32>>,
    tongue_status: TongueStatus,
    processor: Processor,
    mapper: Mapper,
    avatar: Avatar,
    started: Instant,
    last_timestamp: u64,
    camera_fps: FpsCounter,
    tracking_fps: FpsCounter,
    preview_seq: u64,
    last_memory_sample: Instant,
}

impl Tracker {
    fn new(shared: Arc<Shared>, sources: Sources, model_dir: PathBuf, commands: Receiver<Command>) -> Self {
        Self {
            shared,
            sources,
            model_dir,
            commands,
            camera: None,
            camera_key: None,
            landmarker: None,
            face_gpu: None,
            tongue: None,
            tongue_gpu: None,
            tongue_status: TongueStatus::Off,
            processor: Processor::new(),
            mapper: Mapper::new(),
            avatar: Avatar {
                key: None,
                info: AvatarInfo::default(),
                bindings: Vec::new(),
                flags: Vec::new(),
                ft_params: Vec::new(),
                vrchat_found: false,
                live: false,
            },
            started: Instant::now(),
            last_timestamp: 0,
            camera_fps: FpsCounter::new(),
            tracking_fps: FpsCounter::new(),
            preview_seq: 0,
            last_memory_sample: Instant::now() - Duration::from_secs(10),
        }
    }

    fn set_status(&self, status: Status) {
        self.shared.snapshot.lock().status = status;
    }

    fn message(&self, key: &'static str) {
        self.shared.snapshot.lock().message = Some((key, Instant::now()));
    }

    fn handle_commands(&mut self) {
        while let Ok(command) = self.commands.try_recv() {
            let key = self.avatar.info.id.clone().unwrap_or_default();
            match command {
                Command::CaptureNeutral => self.processor.start_neutral_capture(),
                Command::CenterGaze => self.processor.start_gaze_capture(),
                Command::UndoNeutral => {
                    if self.shared.calibrations.lock().avatars.entry(key).or_default().undo_neutral() {
                        self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
                    }
                }
                Command::ResetGaze => {
                    self.shared.calibrations.lock().avatars.entry(key).or_default().gaze_center = None;
                    self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
                }
                Command::ReloadModels => {
                    self.landmarker = None;
                    self.tongue = None;
                    self.tongue_gpu = None;
                    self.tongue_status = TongueStatus::Off;
                }
            }
        }
    }

    fn run(mut self, repaint: impl Fn()) {
        let mut logged = None;
        loop {
            self.handle_commands();
            let status = self.shared.snapshot.lock().status.clone();
            if logged.as_ref() != Some(&status) {
                log::info!("status: {status:?}");
                logged = Some(status);
            }
            if self.last_memory_sample.elapsed() >= Duration::from_secs(5) {
                self.last_memory_sample = Instant::now();
                if let Some(sample) = system::process_memory() {
                    self.shared.perf.lock().sample_memory(sample);
                }
            }
            let settings = self.shared.settings.read().clone();
            self.refresh_avatar(&settings);
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
        let face_gpu = resolve(settings.face_device, &self.shared.gpus);
        if self.landmarker.is_none() || self.face_gpu != Some(face_gpu) {
            self.landmarker = None;
            self.face_gpu = Some(face_gpu);
            let (landmarker, used) = match FaceLandmarker::load(&self.model_dir, face_gpu) {
                Ok(l) => (l, face_gpu),
                Err(e) if face_gpu.is_some() => {
                    log::warn!("GPU face inference unavailable, using CPU: {e:#}");
                    let l = FaceLandmarker::load(&self.model_dir, None)
                        .map_err(|e| Status::ModelError(format!("{e:#}")))?;
                    (l, None)
                }
                Err(e) => return Err(Status::ModelError(format!("{e:#}"))),
            };
            self.shared.snapshot.lock().face_device = device_label(used, &self.shared.gpus);
            self.landmarker = Some(landmarker);
        }
        self.ensure_tongue(settings);
        Ok(())
    }

    fn ensure_tongue(&mut self, settings: &Settings) {
        if !settings.tongue {
            self.tongue = None;
            self.tongue_gpu = None;
            self.tongue_status = TongueStatus::Off;
            return;
        }
        let gpu = resolve(settings.tongue_device, &self.shared.gpus);
        if self.tongue_gpu == Some(gpu) && self.tongue_status != TongueStatus::Off {
            return;
        }
        self.tongue = None;
        self.tongue_gpu = Some(gpu);
        let path = tongue_path(&self.model_dir);
        let load = |gpu: Option<i32>| match &path {
            Some(path) => TongueModel::load(path, gpu),
            #[cfg(embedded_tongue)]
            None => TongueModel::load_embedded(gpu),
            #[cfg(not(embedded_tongue))]
            None => unreachable!("checked below"),
        };
        if path.is_none() && !cfg!(embedded_tongue) {
            self.tongue_status = TongueStatus::Missing;
            return;
        }
        let loaded = load(gpu).map(|m| (m, gpu)).or_else(|e| {
            if gpu.is_none() {
                return Err(e);
            }
            log::warn!("GPU tongue inference unavailable, using CPU: {e:#}");
            load(None).map(|m| (m, None))
        });
        match loaded {
            Ok((model, used)) => {
                self.shared.snapshot.lock().tongue_device = device_label(used, &self.shared.gpus);
                self.tongue = Some(model);
                self.tongue_status = TongueStatus::Ready;
            }
            Err(e) => self.tongue_status = TongueStatus::Error(format!("{e:#}")),
        }
    }

    /// Merges OSCQuery, `/avatar/change` and VRChat's avatar config files.
    fn refresh_avatar(&mut self, settings: &Settings) {
        let (oq_generation, oq_avatar, oq_endpoint) = {
            let d = self.sources.discovery.lock();
            (d.generation, d.avatar.clone(), d.endpoint.is_some())
        };
        let (rx_generation, rx_id) = {
            let r = self.sources.receiver.lock();
            (r.generation, r.avatar_id.clone())
        };
        // With OSCQuery in use the avatar is live while it answers; without it, once VRChat
        // has announced one (the receiver starts with the remembered avatar, at generation 0).
        // Otherwise it is the one remembered from the last session.
        let live = if oq_generation > 0 { oq_avatar.is_some() } else { rx_generation > 0 && rx_id.is_some() };
        let id = oq_avatar.as_ref().and_then(|a| a.id.clone()).or(rx_id).or_else(|| settings.last_avatar_id.clone());
        // Outside the key: closing VRChat changes this without changing the avatar.
        self.avatar.live = live;
        let key = (oq_generation, rx_generation, id.clone(), settings.send_without_avatar, settings.cat_ears);
        if self.avatar.key.as_ref() == Some(&key) {
            return;
        }
        self.avatar.key = Some(key);
        self.avatar.vrchat_found = oq_endpoint || rx_generation > 0;

        let file = id.as_deref().and_then(avatar::load_config);
        let info = match (oq_avatar, file) {
            (Some(mut live), file) => {
                live.name = file.and_then(|f| f.name);
                live
            }
            (None, Some(file)) => file,
            (None, None) => AvatarInfo { id: id.clone(), ..Default::default() },
        };

        let mut bindings = if !info.params.is_empty() {
            osc::bind(&info)
        } else if settings.send_without_avatar {
            osc::default_bindings()
        } else {
            vec![None; OUTPUTS.len()]
        };
        for (binding, output) in bindings.iter_mut().zip(OUTPUTS) {
            if output.cat_ears && !settings.cat_ears {
                *binding = None;
            }
        }
        self.avatar.flags = osc::tracking_flags(&info);
        self.avatar.ft_params = info
            .params
            .iter()
            .filter_map(|p| p.path.rsplit_once("v2/").map(|(_, n)| n.to_string()))
            .filter(|n| !n.is_empty())
            .collect();
        self.avatar.ft_params.sort();
        self.avatar.ft_params.dedup();
        self.avatar.bindings = bindings;
        if info.id.is_some() && info.id != settings.last_avatar_id {
            self.shared.settings.write().last_avatar_id = info.id.clone();
        }
        self.avatar.info = info;

        // Publish now rather than with the next tracked face, so the panel updates
        // even while no face is visible.
        let mut snapshot = self.shared.snapshot.lock();
        snapshot.bindings.clone_from(&self.avatar.bindings);
        snapshot.avatar_id.clone_from(&self.avatar.info.id);
        snapshot.avatar_name.clone_from(&self.avatar.info.name);
        snapshot.avatar_ft_params.clone_from(&self.avatar.ft_params);
        snapshot.vrchat_found = self.avatar.vrchat_found;
        snapshot.avatar_live = self.avatar.live;
    }

    fn step(&mut self, settings: &Settings) -> Result<(), Status> {
        self.ensure_camera(settings)?;
        self.ensure_models(settings)?;

        let read_started = Instant::now();
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
        let captured = Instant::now();
        let perf = self.shared.perf.clone();
        let convert = camera.convert_ms;
        {
            let mut p = perf.lock();
            p.record(Stage::ReadWait, (ms(read_started) - convert).max(0.0));
            p.record(Stage::Convert, convert);
        }
        let camera_fps = self.camera_fps.tick();
        if settings.mirror {
            let started = Instant::now();
            frame.flip_horizontal();
            perf.lock().record(Stage::Mirror, ms(started));
        }

        let timestamp = (self.started.elapsed().as_millis() as u64).max(self.last_timestamp + 1);
        self.last_timestamp = timestamp;
        let inference_started = Instant::now();
        let landmarker = self.landmarker.as_mut().expect("models loaded above");
        let face = landmarker.detect(&frame, timestamp).map_err(|e| Status::ModelError(format!("{e:#}")))?;
        perf.lock().record(Stage::MediaPipe, ms(inference_started));
        let mediapipe_done = Instant::now();

        let preview_started = Instant::now();
        let preview = settings.show_camera.then(|| {
            self.preview_seq += 1;
            let mut small = frame.downscale_to_width(PREVIEW_MAX_WIDTH);
            let revealed = self.shared.unblurred_until.lock().is_some_and(|until| Instant::now() < until);
            if settings.camera_blur && !revealed {
                small.blur(small.width / 60 + 2);
            }
            Arc::new(small)
        });
        let preview_done = preview.is_some().then(|| {
            perf.lock().record(Stage::Preview, ms(preview_started));
            Instant::now()
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
        let mut tongue_done = None;
        if let Some(tongue) = &mut self.tongue {
            let started = Instant::now();
            match tongue.update(&frame, &face.pixel_landmarks) {
                Ok(values) => raw[channels::MEDIAPIPE.len()..].copy_from_slice(&values),
                Err(e) => log::debug!("tongue: {e:#}"),
            }
            perf.lock().record(Stage::Tongue, ms(started));
            tongue_done = Some(Instant::now());
        }

        let processing_started = Instant::now();
        let frown = face::frown_metric(&face.pixel_landmarks);
        let avatar_key = self.avatar.info.id.clone().unwrap_or_default();
        let (processed, outcome) = {
            let mut store = self.shared.calibrations.lock();
            let calibration = store.avatars.entry(avatar_key).or_default();
            self.processor.process(&raw, frown, timestamp, &settings.processing, calibration)
        };
        if let Some(outcome) = outcome {
            self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
            self.message(match outcome {
                CaptureOutcome::NeutralSaved => "neutral_saved",
                CaptureOutcome::GazeSaved => "gaze_saved",
                CaptureOutcome::NeutralRejected => "neutral_rejected",
            });
        }
        perf.lock().record(Stage::Calibration, ms(processing_started));
        let mapping_started = Instant::now();
        let outputs = self.mapper.evaluate(&processed.values, processed.gaze);
        perf.lock().record(Stage::Mapping, ms(mapping_started));

        let handoff_started = Instant::now();
        let mut times = FrameTimes {
            captured,
            mediapipe: mediapipe_done,
            tongue: tongue_done,
            preview: preview_done,
            core: handoff_started,
            published: handoff_started,
        };
        {
            let mut feed = self.sources.feed.lock();
            feed.push(Frame { captured, times, values: outputs.clone() });
            feed.bindings.clone_from(&self.avatar.bindings);
            feed.flags.clone_from(&self.avatar.flags);
            feed.active = settings.ft_output;
            if feed.neutral.len() != OUTPUTS.len() {
                feed.neutral = self.mapper.neutral();
            }
            feed.interpolate = settings.interpolation;
            feed.host.clone_from(&settings.osc_host);
            feed.port = settings.send_port;
        }

        let mut snapshot = self.shared.snapshot.lock();
        snapshot.status = Status::Running;
        snapshot.tracking_fps = tracking_fps;
        snapshot.camera_fps = camera_fps;
        snapshot.frame_size = (frame.width, frame.height);
        if preview.is_some() {
            snapshot.preview = preview;
            snapshot.preview_seq = self.preview_seq;
        }
        snapshot.landmarks = face.landmarks;
        snapshot.input = processed.input;
        snapshot.values = processed.values;
        snapshot.outputs = outputs;
        snapshot.tongue = self.tongue_status.clone();
        snapshot.capture_progress = self.processor.capture_progress();
        times.published = Instant::now();
        snapshot.times = Some(times);
        snapshot.frame_seq += 1;
        drop(snapshot);
        perf.lock().record(Stage::Handoff, ms(handoff_started));
        Ok(())
    }
}
