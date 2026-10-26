//! Webcam capture. Windows uses Media Foundation directly, so we can choose the
//! camera's native mode (usually MJPEG) and let the system decode it to RGB.
//! Other platforms go through nokhwa.

use crate::image::RgbFrame;

#[derive(Clone, Debug)]
pub struct CameraEntry {
    pub index: u32,
    pub name: String,
}

pub use platform::{CameraSource, list};

#[cfg(windows)]
mod platform {
    use std::sync::Once;

    use anyhow::{Context, Result, bail};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
    use windows::core::{GUID, PWSTR};

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

    pub struct CameraSource {
        source: IMFMediaSource,
        reader: IMFSourceReader,
        width: usize,
        height: usize,
        stride: isize,
        /// Time the last frame took to copy into an RGB frame, for the debug view.
        pub convert_ms: f32,
    }

    // The source reader is free-threaded; the camera is only used from the tracker thread.
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
                let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes)?;

                // Prefer modes that reach the requested frame rate, then the closest
                // resolution, then compressed formats that leave USB bandwidth spare.
                let target = (width * height) as i64;
                let rank = |m: &NativeMode| {
                    let format = match m.subtype {
                        s if s == MFVideoFormat_MJPG => 0,
                        s if s == MFVideoFormat_NV12 => 1,
                        s if s == MFVideoFormat_YUY2 => 2,
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

                let output = MFCreateMediaType()?;
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
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
                reader.SetCurrentMediaType(STREAM, None, &output).context("camera mode cannot be converted to RGB")?;

                let current = reader.GetCurrentMediaType(STREAM)?;
                let (w, h) = split(current.GetUINT64(&MF_MT_FRAME_SIZE)?);
                let stride =
                    current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32 as isize).unwrap_or(w as isize * 4);
                log::info!(
                    "camera {index} opened at {w}x{h} {:.0} FPS ({}), {} native modes",
                    mode.fps,
                    subtype_name(mode.subtype),
                    modes.len()
                );
                Ok(Self { source, reader, width: w as usize, height: h as usize, stride, convert_ms: 0.0 })
            }
        }

        pub fn read(&mut self) -> Result<RgbFrame> {
            loop {
                let mut flags = 0u32;
                let mut sample = None;
                unsafe { self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample))? };
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    bail!("camera stream ended");
                }
                // Stream ticks and format changes arrive without a sample.
                let Some(sample) = sample else { continue };
                let started = std::time::Instant::now();
                let frame = unsafe { self.convert(&sample) };
                self.convert_ms = started.elapsed().as_secs_f32() * 1000.0;
                return frame;
            }
        }

        unsafe fn convert(&self, sample: &IMFSample) -> Result<RgbFrame> {
            let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
            let mut data = std::ptr::null_mut();
            let mut len = 0u32;
            unsafe { buffer.Lock(&mut data, None, Some(&mut len))? };
            let row = self.stride.unsigned_abs();
            let needed = row * self.height;
            let result = if (len as usize) < needed || row < self.width * 4 {
                Err(anyhow::anyhow!("short camera frame: {len} < {needed}"))
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
                let mut rgb = Vec::with_capacity(self.width * self.height * 3);
                for y in 0..self.height {
                    let line = if self.stride >= 0 { y } else { self.height - 1 - y };
                    let src = &bytes[line * row..line * row + self.width * 4];
                    for px in src.as_chunks::<4>().0 {
                        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
                    }
                }
                Ok(RgbFrame::new(self.width, self.height, rgb))
            };
            unsafe { buffer.Unlock()? };
            result
        }
    }

    impl Drop for CameraSource {
        fn drop(&mut self) {
            unsafe {
                let _ = self.reader.Flush(MF_SOURCE_READER_ALL_STREAMS.0 as u32);
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
