//! Webcam capture. Windows uses Media Foundation directly: frames come in the camera's
//! own YUV format and `yuv` converts and shrinks them to the requested width. Other
//! platforms go through nokhwa.

#[cfg(windows)]
mod yuv;

use crate::image::RgbFrame;

#[derive(Clone, Debug)]
pub struct CameraEntry {
    pub index: u32,
    pub name: String,
}

pub use platform::{CameraSource, list};

#[cfg(windows)]
mod platform {
    use std::sync::{Arc, Once};
    use std::time::Duration;

    use anyhow::{Context, Result, bail};
    use crossbeam_channel::{Receiver, Sender, TrySendError};
    use parking_lot::Mutex;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
    use windows::core::{GUID, HRESULT, PWSTR, Ref, implement};

    use super::yuv::{self, Layout, Matrix};
    use super::{CameraEntry, RgbFrame};

    const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    fn startup() {
        static STARTUP: Once = Once::new();
        unsafe {
            // Fails harmlessly when the thread already joined another apartment.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            STARTUP.call_once(|| {
                if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                    log::error!("MFStartup failed: {e}");
                }
            });
        }
    }

    fn devices() -> Result<Vec<IMFActivate>> {
        startup();
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.context("no attributes")?;
            attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
            let mut array = std::ptr::null_mut();
            let mut count = 0u32;
            MFEnumDeviceSources(&attributes, &mut array, &mut count)?;
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                if let Some(device) = (*array.add(i)).take() {
                    out.push(device);
                }
            }
            CoTaskMemFree(Some(array as _));
            Ok(out)
        }
    }

    fn friendly_name(device: &IMFActivate) -> String {
        unsafe {
            let mut name = PWSTR::null();
            let mut len = 0u32;
            if device.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut name, &mut len).is_err() {
                return "Camera".into();
            }
            let text = name.to_string().unwrap_or_else(|_| "Camera".into());
            CoTaskMemFree(Some(name.0 as _));
            text
        }
    }

    pub fn list() -> Vec<CameraEntry> {
        match devices() {
            Ok(devices) => devices
                .iter()
                .enumerate()
                .map(|(i, d)| CameraEntry { index: i as u32, name: friendly_name(d) })
                .collect(),
            Err(e) => {
                log::warn!("camera enumeration failed: {e}");
                Vec::new()
            }
        }
    }

    struct NativeMode {
        media_type: IMFMediaType,
        width: u32,
        height: u32,
        fps: f32,
        subtype: GUID,
    }

    fn split(value: u64) -> (u32, u32) {
        ((value >> 32) as u32, value as u32)
    }

    fn subtype_name(subtype: GUID) -> String {
        let fourcc = subtype.data1.to_le_bytes();
        if fourcc.iter().all(|b| b.is_ascii_alphanumeric()) {
            String::from_utf8_lossy(&fourcc).into_owned()
        } else {
            format!("{:#x}", subtype.data1)
        }
    }

    fn native_modes(reader: &IMFSourceReader) -> Vec<NativeMode> {
        let mut modes = Vec::new();
        for i in 0.. {
            let Ok(media_type) = (unsafe { reader.GetNativeMediaType(STREAM, i) }) else { break };
            let mode = unsafe {
                let (width, height) = split(media_type.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0));
                let (num, den) = split(media_type.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0));
                let subtype = media_type.GetGUID(&MF_MT_SUBTYPE).unwrap_or_default();
                NativeMode { media_type, width, height, fps: num as f32 / den.max(1) as f32, subtype }
            };
            modes.push(mode);
        }
        modes
    }

    /// What the reader hands over. Media Foundation objects are free-threaded.
    enum Delivery {
        Frame(IMFSample),
        Ended,
        Failed(HRESULT),
    }

    unsafe impl Send for Delivery {}

    /// The reader, shared with its callback so the callback can ask for the next frame.
    type ReaderSlot = Arc<Mutex<Option<IMFSourceReader>>>;

    /// Takes each frame on Media Foundation's thread and asks for the next one at once, so a
    /// request is always pending while the tracker works on the previous frame.
    #[implement(IMFSourceReaderCallback)]
    struct Callback {
        reader: ReaderSlot,
        frames: Sender<Delivery>,
        /// The other end, to throw away a frame the tracker has not taken yet.
        stale: Receiver<Delivery>,
    }

    impl IMFSourceReaderCallback_Impl for Callback_Impl {
        fn OnReadSample(
            &self,
            status: HRESULT,
            _stream: u32,
            flags: u32,
            _timestamp: i64,
            sample: Ref<IMFSample>,
        ) -> windows::core::Result<()> {
            let delivery = if status.is_err() {
                Some(Delivery::Failed(status))
            } else if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                Some(Delivery::Ended)
            } else {
                // Stream ticks and format changes arrive without a sample.
                sample.ok().ok().cloned().map(Delivery::Frame)
            };
            if !matches!(delivery, Some(Delivery::Ended | Delivery::Failed(_)))
                && let Some(reader) = &*self.reader.lock()
            {
                unsafe { reader.ReadSample(STREAM, 0, None, None, None, None)? };
            }
            if let Some(delivery) = delivery
                && let Err(TrySendError::Full(delivery)) = self.frames.try_send(delivery)
            {
                // Only the newest frame matters.
                let _ = self.stale.try_recv();
                let _ = self.frames.try_send(delivery);
            }
            Ok(())
        }

        fn OnFlush(&self, _stream: u32) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnEvent(&self, _stream: u32, _event: Ref<IMFMediaEvent>) -> windows::core::Result<()> {
            Ok(())
        }
    }

    pub struct CameraSource {
        source: IMFMediaSource,
        reader: ReaderSlot,
        frames: Receiver<Delivery>,
        layout: Layout,
        width: usize,
        height: usize,
        stride: isize,
        /// Frames are shrunk by this whole factor towards the requested width.
        factor: usize,
        matrix: Matrix,
        /// Time the last frame took to convert into an RGB frame, for the debug view.
        pub convert_ms: f32,
    }

    // Media Foundation objects are free-threaded; the camera is only used from the tracker thread.
    unsafe impl Send for CameraSource {}

    impl CameraSource {
        pub fn open(index: u32, (width, height): (u32, u32), fps: u32) -> Result<Self> {
            let device = devices()?.into_iter().nth(index as usize).with_context(|| format!("no camera {index}"))?;
            unsafe {
                let source: IMFMediaSource = device.ActivateObject()?;
                let mut attributes = None;
                MFCreateAttributes(&mut attributes, 1)?;
                let attributes = attributes.context("no attributes")?;
                attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
                let slot = ReaderSlot::default();
                let (tx, rx) = crossbeam_channel::bounded(1);
                let callback: IMFSourceReaderCallback =
                    Callback { reader: slot.clone(), frames: tx, stale: rx.clone() }.into();
                attributes.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
                let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes)?;

                // Prefer modes that reach the requested frame rate, then the closest
                // resolution, then formats that need no decoding.
                let target = (width * height) as i64;
                let rank = |m: &NativeMode| {
                    let format = match m.subtype {
                        s if s == MFVideoFormat_NV12 => 0,
                        s if s == MFVideoFormat_YUY2 => 1,
                        s if s == MFVideoFormat_MJPG => 2,
                        _ => 3,
                    };
                    (m.fps + 0.5 < fps as f32, ((m.width * m.height) as i64 - target).abs(), format, -(m.fps as i32))
                };
                let modes = native_modes(&reader);
                for m in &modes {
                    log::debug!("mode {}x{} {:.1} FPS {}", m.width, m.height, m.fps, subtype_name(m.subtype));
                }
                let mode = modes.iter().min_by_key(|m| rank(m)).context("camera reports no video modes")?;
                reader.SetCurrentMediaType(STREAM, None, &mode.media_type)?;

                // NV12 and YUY2 come as they are, MJPEG decoded to one of them, and `yuv`
                // converts and shrinks them. The video processor's RGB32 path (kept for other
                // formats) cost as much per frame and also made up frames: a camera that slows
                // to 15 FPS in low light came out at 30, every frame twice through the models.
                let yuv_native = [MFVideoFormat_NV12, MFVideoFormat_YUY2].contains(&mode.subtype);
                let output = MFCreateMediaType()?;
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                let yuv = yuv_native
                    || [MFVideoFormat_NV12, MFVideoFormat_YUY2].into_iter().any(|subtype| {
                        output.SetGUID(&MF_MT_SUBTYPE, &subtype).is_ok()
                            && reader.SetCurrentMediaType(STREAM, None, &output).is_ok()
                    });
                if !yuv {
                    output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    // Cameras that only offer large modes are scaled down by the video
                    // processor to the requested width, keeping the native aspect ratio.
                    if mode.width > width && mode.height > 0 {
                        let scaled_height = ((width as u64 * mode.height as u64 / mode.width as u64) as u32 + 1) & !1;
                        output.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | scaled_height as u64)?;
                        if reader.SetCurrentMediaType(STREAM, None, &output).is_err() {
                            output.DeleteItem(&MF_MT_FRAME_SIZE)?;
                        }
                    }
                    reader
                        .SetCurrentMediaType(STREAM, None, &output)
                        .context("camera mode cannot be converted to RGB")?;
                }

                let current = reader.GetCurrentMediaType(STREAM)?;
                let (w, h) = split(current.GetUINT64(&MF_MT_FRAME_SIZE)?);
                let (layout, bytes_per_pixel) = match current.GetGUID(&MF_MT_SUBTYPE)? {
                    s if s == MFVideoFormat_NV12 => (Layout::Nv12, 1),
                    s if s == MFVideoFormat_YUY2 => (Layout::Yuy2, 2),
                    _ => (Layout::Bgra, 4),
                };
                let stride = current
                    .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                    .map(|s| s as i32 as isize)
                    .unwrap_or((w * bytes_per_pixel) as isize);
                // Media Foundation's own default: BT.709 from 720 lines up.
                let matrix = match current.GetUINT32(&MF_MT_YUV_MATRIX) {
                    Ok(m) if m == MFVideoTransferMatrix_BT601.0 as u32 => Matrix::BT601,
                    Ok(m) if m == MFVideoTransferMatrix_BT709.0 as u32 => Matrix::BT709,
                    _ if h >= 720 => Matrix::BT709,
                    _ => Matrix::BT601,
                };
                let factor = if layout == Layout::Bgra { 1 } else { (w / width.max(1)).max(1) as usize };
                log::info!(
                    "camera {index} opened at {w}x{h} {:.0} FPS ({} as {layout:?}, shrunk {factor}x), {} native modes",
                    mode.fps,
                    subtype_name(mode.subtype),
                    modes.len()
                );
                reader.ReadSample(STREAM, 0, None, None, None, None)?;
                *slot.lock() = Some(reader);
                Ok(Self {
                    source,
                    reader: slot,
                    frames: rx,
                    layout,
                    width: w as usize,
                    height: h as usize,
                    stride,
                    factor,
                    matrix,
                    convert_ms: 0.0,
                })
            }
        }

        pub fn read(&mut self) -> Result<RgbFrame> {
            let sample = match self.frames.recv_timeout(Duration::from_secs(2)) {
                Ok(Delivery::Frame(sample)) => sample,
                Ok(Delivery::Ended) => bail!("camera stream ended"),
                Ok(Delivery::Failed(status)) => bail!("camera read failed: {status}"),
                Err(_) => bail!("no frame from the camera for 2 s"),
            };
            let started = std::time::Instant::now();
            let frame = unsafe { self.convert(&sample) };
            self.convert_ms = started.elapsed().as_secs_f32() * 1000.0;
            frame
        }

        unsafe fn convert(&self, sample: &IMFSample) -> Result<RgbFrame> {
            let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
            let mut data = std::ptr::null_mut();
            let mut len = 0u32;
            unsafe { buffer.Lock(&mut data, None, Some(&mut len))? };
            let frame = yuv::Frame {
                layout: self.layout,
                data: unsafe { std::slice::from_raw_parts(data, len as usize) },
                width: self.width,
                height: self.height,
                stride: self.stride,
            };
            let mut rgb = Vec::new();
            let size = yuv::to_rgb(&frame, self.factor, self.matrix, &mut rgb);
            unsafe { buffer.Unlock()? };
            let (width, height) = size.with_context(|| format!("short camera frame: {len} bytes"))?;
            Ok(RgbFrame::new(width, height, rgb))
        }
    }

    impl Drop for CameraSource {
        fn drop(&mut self) {
            // Taking the reader out of the callback's slot also ends the cycle between them.
            if let Some(reader) = self.reader.lock().take() {
                unsafe {
                    let _ = reader.Flush(MF_SOURCE_READER_ALL_STREAMS.0 as u32);
                }
            }
            unsafe {
                let _ = self.source.Shutdown();
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use anyhow::{Context, Result};
    use nokhwa::Camera;
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    };

    use super::{CameraEntry, RgbFrame};

    pub fn list() -> Vec<CameraEntry> {
        match nokhwa::query(ApiBackend::Auto) {
            Ok(cameras) => cameras
                .into_iter()
                .filter_map(|info| match info.index() {
                    CameraIndex::Index(index) => Some(CameraEntry { index: *index, name: info.human_name() }),
                    CameraIndex::String(_) => None,
                })
                .collect(),
            Err(e) => {
                log::warn!("camera query failed: {e}");
                Vec::new()
            }
        }
    }

    pub struct CameraSource {
        camera: Camera,
        /// Decoding happens inside `read`; reported as part of the read time.
        pub convert_ms: f32,
    }

    impl CameraSource {
        pub fn open(index: u32, (width, height): (u32, u32), fps: u32) -> Result<Self> {
            let attempts = [
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(width, height),
                    FrameFormat::MJPEG,
                    fps,
                )),
                RequestedFormatType::Closest(CameraFormat::new(Resolution::new(width, height), FrameFormat::YUYV, fps)),
                RequestedFormatType::AbsoluteHighestFrameRate,
            ];
            let mut last_error = None;
            for request in attempts {
                match Camera::new(CameraIndex::Index(index), RequestedFormat::new::<RgbFormat>(request)) {
                    Ok(mut camera) => {
                        camera.open_stream().context("opening camera stream")?;
                        log::info!("camera {index} opened as {}", camera.camera_format());
                        return Ok(Self { camera, convert_ms: 0.0 });
                    }
                    Err(e) => last_error = Some(e),
                }
            }
            Err(last_error.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("no camera format")))
                .with_context(|| format!("opening camera {index}"))
        }

        pub fn read(&mut self) -> Result<RgbFrame> {
            let buffer = self.camera.frame()?;
            let image = buffer.decode_image::<RgbFormat>()?;
            let (width, height) = image.dimensions();
            Ok(RgbFrame::new(width as usize, height as usize, image.into_raw()))
        }
    }

    impl Drop for CameraSource {
        fn drop(&mut self) {
            let _ = self.camera.stop_stream();
        }
    }
}
