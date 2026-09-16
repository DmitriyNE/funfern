use bevy::platform::time::Instant;
use bevy_egui::egui::Rect;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

pub(crate) const VIDEO_FPS: u32 = 60;

/// How many window readbacks may be outstanding at once while recording.
///
/// A readback's round trip is far longer than a rendered frame — measured at
/// about 35 ms against 8 ms of frame time on an M1 Max — so gating on a single
/// one in flight caps capture at roughly 28 distinct frames a second whatever
/// rate is asked for. Three in flight covers 57 of every 60 slots on the same
/// machine with no measurable change to frame time; the slots that still miss
/// are filled from the previous frame, so the file stays wall-clock true.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_READBACKS_IN_FLIGHT: usize = 3;

/// The longest run of missed slots the encoder fills from the previous frame,
/// as a duration rather than a frame count so it means the same at any rate.
#[cfg(not(target_arch = "wasm32"))]
const MAX_HELD_FRAME: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct RecordingSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug)]
pub(crate) enum RecordingEvent {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    DestinationReady,
    Started(String),
    Finished(String),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Cancelled,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    DroppedFrame,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DestinationRequest {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Ready,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Pending,
}

pub(crate) struct VideoRecorder {
    events_rx: Mutex<Receiver<RecordingEvent>>,
    backend: backend::Backend,
}

impl Default for VideoRecorder {
    fn default() -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            backend: backend::Backend::new(events_tx.clone()),
            events_rx: Mutex::new(events_rx),
        }
    }
}

impl VideoRecorder {
    pub fn request_destination(&mut self) -> Result<DestinationRequest, String> {
        self.backend.request_destination()
    }

    pub fn start(
        &mut self,
        spec: RecordingSpec,
        logical_canvas: Rect,
        logical_viewport: Rect,
    ) -> Result<(), String> {
        self.backend.start(spec, logical_canvas, logical_viewport)
    }

    pub fn update_viewport(&mut self, logical_canvas: Rect, logical_viewport: Rect) {
        self.backend
            .update_viewport(logical_canvas, logical_viewport);
    }

    pub fn stop(&mut self) {
        self.backend.stop();
    }

    pub fn poll(&mut self) -> Vec<RecordingEvent> {
        self.backend.poll();
        self.events_rx.lock().unwrap().try_iter().collect()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn native_frame_target(&self) -> Option<backend::NativeFrameTarget> {
        self.backend.frame_target()
    }
}

pub(crate) fn elapsed_label(started: Instant) -> String {
    let seconds = started.elapsed().as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use super::*;
    use bevy::prelude::Image;
    use std::{
        io::Write,
        path::PathBuf,
        process::{Command, Stdio},
        sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError},
        thread,
        time::Duration,
    };

    #[derive(Clone, Debug)]
    struct EncoderChoice {
        codec: &'static str,
        extension: &'static str,
        description: &'static str,
    }

    #[derive(Clone, Debug)]
    struct PreparedDestination {
        path: PathBuf,
        encoder: EncoderChoice,
    }

    type PreparationResult = Result<Option<PreparedDestination>, String>;

    pub(crate) struct NativeFrame {
        image: Image,
        logical_canvas: Rect,
        logical_viewport: Rect,
        slot: u64,
    }

    #[derive(Clone)]
    pub(crate) struct NativeFrameTarget {
        frames: SyncSender<NativeFrame>,
        events: Sender<RecordingEvent>,
    }

    impl NativeFrameTarget {
        pub fn submit(
            &self,
            image: Image,
            logical_canvas: Rect,
            logical_viewport: Rect,
            slot: u64,
        ) {
            let frame = NativeFrame {
                image,
                logical_canvas,
                logical_viewport,
                slot,
            };
            if matches!(self.frames.try_send(frame), Err(TrySendError::Full(_))) {
                let _ = self.events.send(RecordingEvent::DroppedFrame);
            }
        }
    }

    pub(crate) struct Backend {
        events: Sender<RecordingEvent>,
        preparation: Option<Mutex<Receiver<PreparationResult>>>,
        prepared: Option<PreparedDestination>,
        frames: Option<SyncSender<NativeFrame>>,
        stop: Option<Sender<()>>,
    }

    impl Backend {
        pub fn new(events: Sender<RecordingEvent>) -> Self {
            Self {
                events,
                preparation: None,
                prepared: None,
                frames: None,
                stop: None,
            }
        }

        pub fn request_destination(&mut self) -> Result<DestinationRequest, String> {
            if self.preparation.is_some() || self.frames.is_some() {
                return Err("A recording is already being prepared".into());
            }
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let result = probe_encoder().map(|encoder| {
                    let path = rfd::FileDialog::new()
                        .add_filter(encoder.description, &[encoder.extension])
                        .set_file_name(format!("funfern-recording.{}", encoder.extension))
                        .save_file();
                    path.map(|mut path| {
                        if path.extension().and_then(|value| value.to_str())
                            != Some(encoder.extension)
                        {
                            path.set_extension(encoder.extension);
                        }
                        PreparedDestination { path, encoder }
                    })
                });
                let _ = sender.send(result);
            });
            self.preparation = Some(Mutex::new(receiver));
            Ok(DestinationRequest::Pending)
        }

        pub fn poll(&mut self) {
            let result = self
                .preparation
                .as_ref()
                .and_then(|receiver| receiver.lock().unwrap().try_recv().ok());
            if let Some(result) = result {
                self.preparation = None;
                match result {
                    Ok(Some(destination)) => {
                        self.prepared = Some(destination);
                        let _ = self.events.send(RecordingEvent::DestinationReady);
                    }
                    Ok(None) => {
                        let _ = self.events.send(RecordingEvent::Cancelled);
                    }
                    Err(error) => {
                        let _ = self.events.send(RecordingEvent::Error(error));
                    }
                }
            }
        }

        pub fn start(
            &mut self,
            spec: RecordingSpec,
            _logical_canvas: Rect,
            _logical_viewport: Rect,
        ) -> Result<(), String> {
            let destination = self
                .prepared
                .take()
                .ok_or_else(|| "Recording destination is not ready".to_owned())?;
            // One slot per readback that can be in flight, plus room for the
            // frame the encoder is busy converting.
            let (frame_sender, frame_receiver) = mpsc::sync_channel(MAX_READBACKS_IN_FLIGHT + 2);
            let (stop_sender, stop_receiver) = mpsc::channel();
            let events = self.events.clone();
            thread::spawn(move || {
                encode_video(destination, spec, frame_receiver, stop_receiver, events)
            });
            self.frames = Some(frame_sender);
            self.stop = Some(stop_sender);
            Ok(())
        }

        pub fn update_viewport(&mut self, _logical_canvas: Rect, _logical_viewport: Rect) {}

        pub fn stop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            self.frames = None;
        }

        pub fn frame_target(&self) -> Option<NativeFrameTarget> {
            Some(NativeFrameTarget {
                frames: self.frames.clone()?,
                events: self.events.clone(),
            })
        }
    }

    fn probe_encoder() -> Result<EncoderChoice, String> {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .map_err(|_| {
                "Native recording requires FFmpeg. Install it and make sure `ffmpeg` is on PATH."
                    .to_owned()
            })?;
        if !output.status.success() {
            return Err("Could not inspect the installed FFmpeg encoders".into());
        }
        let encoders = String::from_utf8_lossy(&output.stdout);
        if has_encoder(&encoders, "libx264") {
            Ok(EncoderChoice {
                codec: "libx264",
                extension: "mp4",
                description: "MP4 video",
            })
        } else if has_encoder(&encoders, "h264_videotoolbox") {
            Ok(EncoderChoice {
                codec: "h264_videotoolbox",
                extension: "mp4",
                description: "MP4 video",
            })
        } else if has_encoder(&encoders, "libvpx-vp9") {
            Ok(EncoderChoice {
                codec: "libvpx-vp9",
                extension: "webm",
                description: "WebM video",
            })
        } else if has_encoder(&encoders, "libvpx") {
            Ok(EncoderChoice {
                codec: "libvpx",
                extension: "webm",
                description: "WebM video",
            })
        } else {
            Err("FFmpeg has no supported H.264, VP9, or VP8 encoder".into())
        }
    }

    fn has_encoder(list: &str, name: &str) -> bool {
        list.lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    }

    /// How many slots the encoder will fill from the previous frame before it
    /// gives up on a stall, at `fps`. Derived from [`MAX_HELD_FRAME`] so the
    /// limit means the same length of frozen video at any rate.
    fn held_frames(fps: u32) -> u64 {
        (MAX_HELD_FRAME.as_secs() * u64::from(fps.max(1))).max(1)
    }

    enum SlotAction {
        /// The frame belongs behind one already written. Its slot was filled
        /// from the frame before it, so writing it now would push every later
        /// frame one slot late — and no time is lost by leaving it out, which
        /// is why this is not counted as a dropped frame.
        Skip,
        Write {
            /// Slots missed since the last write, to fill from the held frame.
            duplicates: u64,
        },
    }

    /// Several readbacks are in flight at once and they do not always finish in
    /// the order they were asked for, so the written slot only ever moves
    /// forward.
    fn slot_action(written: Option<u64>, slot: u64, held: u64) -> SlotAction {
        match written {
            Some(written) if slot <= written => SlotAction::Skip,
            Some(written) => SlotAction::Write {
                duplicates: (slot - written - 1).min(held),
            },
            None => SlotAction::Write { duplicates: 0 },
        }
    }

    fn encode_video(
        destination: PreparedDestination,
        spec: RecordingSpec,
        frames: Receiver<NativeFrame>,
        stop: Receiver<()>,
        events: Sender<RecordingEvent>,
    ) {
        let size = format!("{}x{}", spec.width, spec.height);
        let rate = spec.fps.to_string();
        let path = destination.path.to_string_lossy().into_owned();
        let mut command = Command::new("ffmpeg");
        command.args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &size,
            "-framerate",
            &rate,
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            destination.encoder.codec,
        ]);
        if destination.encoder.extension == "mp4" {
            command.args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
            if destination.encoder.codec == "libx264" {
                command.args(["-preset", "veryfast", "-crf", "20"]);
            }
        } else {
            command.args([
                "-deadline",
                "realtime",
                "-cpu-used",
                "4",
                "-crf",
                "30",
                "-b:v",
                "0",
            ]);
        }
        let child = command
            .arg(&destination.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                let _ = events.send(RecordingEvent::Error(format!(
                    "Could not start FFmpeg: {error}"
                )));
                return;
            }
        };
        let mut stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = events.send(RecordingEvent::Error(
                    "FFmpeg did not open its video input".into(),
                ));
                let _ = child.kill();
                return;
            }
        };
        let _ = events.send(RecordingEvent::Started(format!(
            "{}×{} · {} · {} FPS",
            spec.width, spec.height, destination.encoder.description, spec.fps
        )));

        let mut previous: Option<Vec<u8>> = None;
        let mut written: Option<u64> = None;
        let held = held_frames(spec.fps);
        let result = 'encoding: loop {
            if stop.try_recv().is_ok() {
                break Ok(());
            }
            match frames.recv_timeout(Duration::from_millis(20)) {
                Ok(frame) => {
                    let SlotAction::Write { duplicates } = slot_action(written, frame.slot, held)
                    else {
                        continue;
                    };
                    let pixels = match crate::capture::viewport_rgba_frame(
                        frame.image,
                        frame.logical_canvas,
                        frame.logical_viewport,
                        spec.width,
                        spec.height,
                    ) {
                        Ok(pixels) => pixels,
                        Err(error) => break Err(error),
                    };
                    if let Some(previous) = &previous {
                        for _ in 0..duplicates {
                            if let Err(error) = stdin.write_all(previous) {
                                break 'encoding Err(format!(
                                    "Could not write video frame: {error}"
                                ));
                            }
                        }
                    }
                    if let Err(error) = stdin.write_all(&pixels) {
                        break Err(format!("Could not write video frame: {error}"));
                    }
                    previous = Some(pixels);
                    written = Some(frame.slot);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Ok(()),
            }
        };
        drop(stdin);
        let status = child.wait();
        match (result, status) {
            (Ok(()), Ok(status)) if status.success() => {
                let _ = events.send(RecordingEvent::Finished(format!(
                    "Recording saved to {path}"
                )));
            }
            (Err(error), _) => {
                let _ = events.send(RecordingEvent::Error(error));
            }
            (_, Ok(status)) => {
                let _ = events.send(RecordingEvent::Error(format!(
                    "FFmpeg exited with {status}"
                )));
            }
            (_, Err(error)) => {
                let _ = events.send(RecordingEvent::Error(format!(
                    "Could not finalize recording: {error}"
                )));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use image::{DynamicImage, Rgba, RgbaImage};

        /// The cap on a frozen run is a duration, so it has to hold the same
        /// length of video at any rate rather than the same frame count.
        #[test]
        fn a_held_run_is_the_same_length_of_video_at_any_rate() {
            assert_eq!(held_frames(30), 150);
            assert_eq!(held_frames(60), 300);
            // A nonsense rate is read as one frame a second, not as none.
            assert_eq!(held_frames(0), 5);
        }

        /// Readbacks finish out of order often enough to see in a ten-second
        /// recording, and writing a late one would push every frame after it
        /// one slot behind wall clock.
        #[test]
        fn a_frame_behind_the_written_slot_is_left_out() {
            assert!(matches!(slot_action(Some(7), 6, 300), SlotAction::Skip));
            assert!(matches!(slot_action(Some(7), 7, 300), SlotAction::Skip));
            assert!(matches!(
                slot_action(Some(7), 8, 300),
                SlotAction::Write { duplicates: 0 }
            ));
        }

        #[test]
        fn missed_slots_are_filled_from_the_held_frame_and_capped() {
            assert!(matches!(
                slot_action(None, 5, 300),
                SlotAction::Write { duplicates: 0 }
            ));
            assert!(matches!(
                slot_action(Some(4), 9, 300),
                SlotAction::Write { duplicates: 4 }
            ));
            assert!(matches!(
                slot_action(Some(0), 10_000, 300),
                SlotAction::Write { duplicates: 300 }
            ));
        }

        #[test]
        fn encoder_detection_matches_whole_names() {
            let encoders = " V....D libx264rgb RGB only\n V....D libvpx-vp9 VP9\n";
            assert!(!has_encoder(encoders, "libx264"));
            assert!(has_encoder(encoders, "libvpx-vp9"));
            assert!(!has_encoder(encoders, "libvpx"));
        }

        #[test]
        fn installed_ffmpeg_accepts_the_native_frame_stream() {
            let Ok(encoder) = probe_encoder() else {
                return;
            };
            let mut path = std::env::temp_dir();
            path.push(format!(
                "funfern-recording-test-{}.{}",
                std::process::id(),
                encoder.extension
            ));
            let destination = PreparedDestination {
                path: path.clone(),
                encoder,
            };
            let (frames_tx, frames_rx) = mpsc::sync_channel(MAX_READBACKS_IN_FLIGHT + 2);
            let (stop_tx, stop_rx) = mpsc::channel();
            let (events_tx, events_rx) = mpsc::channel();
            let rect = Rect::from_min_max((0.0, 0.0).into(), (16.0, 8.0).into());
            // Slot 1 arrives after slot 2, as a pipelined readback can.
            for (slot, color) in [
                (0_u64, [20, 80, 140, 255]),
                (2, [140, 80, 20, 255]),
                (1, [80, 140, 20, 255]),
            ] {
                let pixels = RgbaImage::from_pixel(16, 8, Rgba(color));
                let image =
                    Image::from_dynamic(DynamicImage::ImageRgba8(pixels), true, Default::default());
                frames_tx
                    .send(NativeFrame {
                        image,
                        logical_canvas: rect,
                        logical_viewport: rect,
                        slot,
                    })
                    .unwrap();
            }
            drop(frames_tx);
            drop(stop_tx);
            encode_video(
                destination,
                RecordingSpec {
                    width: 16,
                    height: 8,
                    fps: VIDEO_FPS,
                },
                frames_rx,
                stop_rx,
                events_tx,
            );
            let events: Vec<_> = events_rx.try_iter().collect();
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, RecordingEvent::Started(_)))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, RecordingEvent::Finished(_)))
            );
            assert!(std::fs::metadata(&path).unwrap().len() > 0);
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    use super::*;
    use std::cell::RefCell;
    use wasm_bindgen::{JsCast, closure::Closure, prelude::*};

    #[wasm_bindgen(inline_js = r#"
let session = null;

function supportedType() {
  if (!window.MediaRecorder || !HTMLCanvasElement.prototype.captureStream) return null;
  const types = [
    ['video/webm;codecs=vp9', 'webm'],
    ['video/webm;codecs=vp8', 'webm'],
    ['video/webm', 'webm'],
    ['video/mp4', 'mp4'],
  ];
  for (const entry of types) {
    if (MediaRecorder.isTypeSupported(entry[0])) return entry;
  }
  return null;
}

export function funfernRecordingSupported() { return supportedType() !== null; }

export function funfernStartRecording(width, height, fps, left, top, right, bottom,
                                      started, finished, failed) {
  if (session) throw new Error('A recording is already active');
  const source = document.getElementById('funfern');
  if (!(source instanceof HTMLCanvasElement)) throw new Error('Could not find the funfern canvas');
  const format = supportedType();
  if (!format) throw new Error('This browser does not support canvas video recording');
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext('2d', { alpha: false });
  if (!context) throw new Error('Could not create the recording canvas');
  const stream = canvas.captureStream(fps);
  const recorder = new MediaRecorder(stream, { mimeType: format[0], videoBitsPerSecond: 6000000 });
  const chunks = [];
  session = { source, canvas, context, stream, recorder, chunks, format, failed,
              crop: { left, top, right, bottom }, frame: 0, running: true };
  const abort = message => {
    const s = session;
    session = null;
    if (!s) return;
    s.running = false;
    cancelAnimationFrame(s.frame);
    s.recorder.onstop = null;
    if (s.recorder.state !== 'inactive') s.recorder.stop();
    for (const track of s.stream.getTracks()) track.stop();
    s.failed(message);
  };
  const draw = () => {
    if (!session || !session.running) return;
    const s = session;
    const sx = Math.max(0, Math.ceil(s.crop.left * s.source.width));
    const sy = Math.max(0, Math.ceil(s.crop.top * s.source.height));
    const sr = Math.min(s.source.width, Math.floor(s.crop.right * s.source.width));
    const sb = Math.min(s.source.height, Math.floor(s.crop.bottom * s.source.height));
    const sw = Math.max(1, sr - sx);
    const sh = Math.max(1, sb - sy);
    const scale = Math.min(s.canvas.width / sw, s.canvas.height / sh);
    const dw = Math.max(1, Math.round(sw * scale));
    const dh = Math.max(1, Math.round(sh * scale));
    const dx = Math.floor((s.canvas.width - dw) / 2);
    const dy = Math.floor((s.canvas.height - dh) / 2);
    s.context.fillStyle = '#0b1117';
    s.context.fillRect(0, 0, s.canvas.width, s.canvas.height);
    try {
      s.context.drawImage(s.source, sx, sy, sw, sh, dx, dy, dw, dh);
    } catch (error) {
      abort(String(error));
      return;
    }
    s.frame = requestAnimationFrame(draw);
  };
  recorder.ondataavailable = event => { if (event.data && event.data.size) chunks.push(event.data); };
  recorder.onerror = event => abort(event.error ? event.error.message : 'Browser video encoder failed');
  recorder.onstop = () => {
    const s = session;
    session = null;
    if (!s) return;
    cancelAnimationFrame(s.frame);
    for (const track of s.stream.getTracks()) track.stop();
    try {
      const blob = new Blob(s.chunks, { type: s.format[0] });
      if (!blob.size) throw new Error('The browser produced an empty recording');
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = `funfern-recording.${s.format[1]}`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      finished(`Recording downloaded · ${s.format[0]}`);
    } catch (error) { failed(String(error)); }
  };
  try {
    recorder.start(1000);
    session.frame = requestAnimationFrame(draw);
  } catch (error) {
    const s = session;
    session = null;
    for (const track of s.stream.getTracks()) track.stop();
    throw error;
  }
  started(`${width}×${height} · ${format[0]} · ${fps} FPS`);
}

export function funfernUpdateRecordingCrop(left, top, right, bottom) {
  if (session) session.crop = { left, top, right, bottom };
}

export function funfernStopRecording() {
  if (!session) return;
  session.running = false;
  cancelAnimationFrame(session.frame);
  if (session.recorder.state !== 'inactive') session.recorder.stop();
}
"#)]
    extern "C" {
        #[wasm_bindgen(js_name = funfernRecordingSupported)]
        fn recording_supported() -> bool;
        #[wasm_bindgen(js_name = funfernStartRecording, catch)]
        fn start_recording(
            width: u32,
            height: u32,
            fps: u32,
            left: f64,
            top: f64,
            right: f64,
            bottom: f64,
            started: &js_sys::Function,
            finished: &js_sys::Function,
            failed: &js_sys::Function,
        ) -> Result<(), JsValue>;
        #[wasm_bindgen(js_name = funfernUpdateRecordingCrop)]
        fn update_recording_crop(left: f64, top: f64, right: f64, bottom: f64);
        #[wasm_bindgen(js_name = funfernStopRecording)]
        fn stop_recording();
    }

    struct Callbacks {
        _started: Closure<dyn FnMut(String)>,
        _finished: Closure<dyn FnMut(String)>,
        _failed: Closure<dyn FnMut(String)>,
    }

    thread_local! {
        static CALLBACKS: RefCell<Option<Callbacks>> = const { RefCell::new(None) };
    }

    pub(crate) struct Backend {
        events: Sender<RecordingEvent>,
    }

    impl Backend {
        pub fn new(events: Sender<RecordingEvent>) -> Self {
            Self { events }
        }

        pub fn request_destination(&mut self) -> Result<DestinationRequest, String> {
            if recording_supported() {
                Ok(DestinationRequest::Ready)
            } else {
                Err("This browser does not support canvas video recording".into())
            }
        }

        pub fn poll(&mut self) {}

        pub fn start(
            &mut self,
            spec: RecordingSpec,
            logical_canvas: Rect,
            logical_viewport: Rect,
        ) -> Result<(), String> {
            let crop = normalized_crop(logical_canvas, logical_viewport)?;
            let started_sender = self.events.clone();
            let finished_sender = self.events.clone();
            let failed_sender = self.events.clone();
            let callbacks = Callbacks {
                _started: Closure::new(move |description: String| {
                    let _ = started_sender.send(RecordingEvent::Started(description));
                }),
                _finished: Closure::new(move |message: String| {
                    let _ = finished_sender.send(RecordingEvent::Finished(message));
                }),
                _failed: Closure::new(move |error: String| {
                    let _ = failed_sender.send(RecordingEvent::Error(error));
                }),
            };
            start_recording(
                spec.width,
                spec.height,
                spec.fps,
                crop.0,
                crop.1,
                crop.2,
                crop.3,
                callbacks._started.as_ref().unchecked_ref(),
                callbacks._finished.as_ref().unchecked_ref(),
                callbacks._failed.as_ref().unchecked_ref(),
            )
            .map_err(js_error)?;
            CALLBACKS.with(|stored| *stored.borrow_mut() = Some(callbacks));
            Ok(())
        }

        pub fn update_viewport(&mut self, logical_canvas: Rect, logical_viewport: Rect) {
            if let Ok(crop) = normalized_crop(logical_canvas, logical_viewport) {
                update_recording_crop(crop.0, crop.1, crop.2, crop.3);
            }
        }

        pub fn stop(&mut self) {
            stop_recording();
        }
    }

    fn normalized_crop(canvas: Rect, viewport: Rect) -> Result<(f64, f64, f64, f64), String> {
        if !canvas.is_finite()
            || !viewport.is_finite()
            || canvas.width() <= 0.0
            || canvas.height() <= 0.0
        {
            return Err("Captured viewport has invalid dimensions".into());
        }
        Ok((
            ((viewport.left() - canvas.left()) / canvas.width()) as f64,
            ((viewport.top() - canvas.top()) / canvas.height()) as f64,
            ((viewport.right() - canvas.left()) / canvas.width()) as f64,
            ((viewport.bottom() - canvas.top()) / canvas.height()) as f64,
        ))
    }

    fn js_error(error: JsValue) -> String {
        error
            .as_string()
            .unwrap_or_else(|| "Could not start browser recording".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_time_is_clock_shaped() {
        assert_eq!(elapsed_label(Instant::now()), "00:00");
    }
}
