//! Interactive latency trace (#1343): where each frame's time goes during native brush strokes and
//! SaiLineArt point drags, so a tester can report a per-stage breakdown instead of one number.
//!
//! Set `PHOTOCRAFT_STROKE_TRACE` before launching: `1` prints a summary to stderr when each stroke
//! ends; a file path also appends every frame of every stroke to that CSV. The last summary is in
//! the control channel's `perf` output (`strokeTrace`).
//!
//! Per frame of a gesture it records the interval since the previous frame started, pointer moves,
//! native live-stroke time, SaiLineArt input/command/cache stages, canvas refresh and texture uploads,
//! the UI build, and eframe's CPU time for the frame before (UI, tessellation and GPU
//! submission, without the vsync wait). What the app can't see (the OS input queue before a frame
//! starts, frames queued for the display, the compositor) is the rest of the input-to-photon time;
//! the summary names the GPU backend because GL ignores the frame-latency setting.

use std::{fmt::Write as _, time::Instant};

/// One frame of a stroke. Times in ms.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Row {
    /// Frame start, from [`crate::gpu_canvas::now_ms`].
    pub t: f64,
    /// Since the previous frame started (0 for the first frame the trace saw).
    pub interval: f64,
    /// Pointer moves the frame delivered.
    pub moves: usize,
    /// Raw-input hook and app logic CPU time for this frame.
    pub raw_hook: f64,
    pub logic: f64,
    /// Rendering the stroke's new points (the live stroke, or the commit on release).
    pub stroke: f64,
    pub native_input_age: f64,
    pub native_updates: u64,
    /// SaiLineArt event-to-handler delay, synchronous move command and pointer age after it.
    pub sailine_input: f64,
    pub sailine_command: f64,
    pub sailine_pointer_age: f64,
    /// Engine refresh stages (centerline interpolation + contour construction are one vector call).
    pub sailine_snapshot_clone: f64,
    pub sailine_shape: f64,
    pub sailine_raster: f64,
    pub sailine_path_cache: f64,
    pub sailine_damage_bounds: f64,
    pub sailine_composition: f64,
    pub sailine_writeback: f64,
    pub sailine_refresh: f64,
    pub sailine_events: u64,
    pub sailine_updates: u64,
    /// Time from the latest interactive update to a canvas texture being ready for drawing.
    pub canvas_ready_ms: f64,
    /// Age of the pointer position when the canvas cache is ready, and when App::ui hands off.
    pub canvas_ready_pointer_age: f64,
    pub handoff_pointer_age: f64,
    /// Canvas refresh: composite and upload.
    pub canvas: f64,
    pub canvas_composite: f64,
    pub canvas_upload: f64,
    pub refreshes: u64,
    pub refreshed_pixels: u64,
    pub gpu_uploads: u64,
    /// Building the UI (`App::ui`), the two above included.
    pub ui: f64,
    /// eframe's CPU time for the previous frame: UI, tessellation and GPU submission.
    pub cpu_prev: Option<f64>,
    /// Approximate CPU time after App::ui returns, from eframe's completed-frame report.
    pub cpu_after_ui: Option<f64>,
    pub estimated_submit_age: Option<f64>,
}

/// Percentiles of a stroke's frames, for the summary line and the control channel.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub frames: usize,
    /// GPU backend and adapter ("vulkan NVIDIA GeForce …").
    pub backend: String,
    /// `[p50, p95]` of each stage, in ms.
    pub interval: [f64; 2],
    pub fps: [f64; 2],
    pub moves: [f64; 2],
    pub stroke: [f64; 2],
    pub native_input_age: [f64; 2],
    pub native_updates: [f64; 2],
    pub sailine_input: [f64; 2],
    pub sailine_command: [f64; 2],
    pub sailine_pointer_age: [f64; 2],
    pub sailine_snapshot_clone: [f64; 2],
    pub sailine_shape: [f64; 2],
    pub sailine_raster: [f64; 2],
    pub sailine_path_cache: [f64; 2],
    pub sailine_damage_bounds: [f64; 2],
    pub sailine_composition: [f64; 2],
    pub sailine_writeback: [f64; 2],
    pub sailine_refresh: [f64; 2],
    pub sailine_events: [f64; 2],
    pub sailine_updates: [f64; 2],
    pub canvas_ready_ms: [f64; 2],
    pub canvas_ready_pointer_age: [f64; 2],
    pub handoff_pointer_age: [f64; 2],
    pub estimated_submit_age: [f64; 2],
    pub cpu_after_ui: [f64; 2],
    pub canvas: [f64; 2],
    pub canvas_composite: [f64; 2],
    pub canvas_upload: [f64; 2],
    pub refreshes: [f64; 2],
    pub refreshed_pixels: [f64; 2],
    pub gpu_uploads: [f64; 2],
    pub ui: [f64; 2],
    /// eframe's CPU time per frame (UI, tessellation, submission).
    pub cpu: [f64; 2],
    /// Frame interval minus that CPU time: waiting on vsync / the swap chain, or idle.
    pub wait: [f64; 2],
}

impl Summary {
    /// One line for stderr.
    pub fn line(&self) -> String {
        let f = |name: &str, [p50, p95]: [f64; 2]| format!("{name} {p50:.2}/{p95:.2}");
        [
            format!("stroke trace: {} frames on {} (p50/p95 ms):", self.frames, self.backend),
            f("interval", self.interval),
            f("fps", self.fps),
            f("moves/frame", self.moves),
            f("stroke", self.stroke),
            f("brush-input-age", self.native_input_age),
            f("brush-updates/frame", self.native_updates),
            f("sai-input", self.sailine_input),
            f("sai-command", self.sailine_command),
            f("sai-pointer-age", self.sailine_pointer_age),
            f("sai-snapshot-copy", self.sailine_snapshot_clone),
            f("sai-shape", self.sailine_shape),
            f("sai-raster", self.sailine_raster),
            f("sai-path-cache", self.sailine_path_cache),
            f("sai-damage-bounds", self.sailine_damage_bounds),
            f("sai-composition", self.sailine_composition),
            f("sai-writeback", self.sailine_writeback),
            f("sai-refresh", self.sailine_refresh),
            f("sai-events/frame", self.sailine_events),
            f("sai-updates/frame", self.sailine_updates),
            f("canvas-ready", self.canvas_ready_ms),
            f("canvas-ready-pointer-age", self.canvas_ready_pointer_age),
            f("ui-handoff-pointer-age", self.handoff_pointer_age),
            f("submit-age-estimate", self.estimated_submit_age),
            f("cpu-after-ui-estimate", self.cpu_after_ui),
            f("canvas", self.canvas),
            f("canvas-composite", self.canvas_composite),
            f("texture-upload", self.canvas_upload),
            f("refreshes/frame", self.refreshes),
            f("pixels/frame", self.refreshed_pixels),
            f("gpu-uploads/frame", self.gpu_uploads),
            f("ui", self.ui),
            f("cpu", self.cpu),
            f("wait", self.wait),
        ]
        .join(" ")
    }
}

/// Where the trace goes ([`StrokeTrace::from_env`]).
// The web build has no clock, so it never traces.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Output {
    #[default]
    Off,
    /// A summary per stroke on stderr.
    Stderr,
    /// The summary on stderr, and every frame appended to this CSV.
    Csv(std::path::PathBuf),
}

#[derive(Debug, Default)]
pub struct StrokeTrace {
    pub(crate) output: Output,
    /// The frame being built (started by `begin_frame`).
    frame: Option<Row>,
    last_start: Option<f64>,
    /// The current stroke's frames.
    rows: Vec<Row>,
    /// Strokes finished so far (numbers the CSV rows).
    strokes: usize,
    /// The last finished stroke's summary.
    pub last: Option<Summary>,
    /// Latest raw pointer position delivered by eframe (egui events contain no OS timestamp).
    last_pointer_received: Option<Instant>,
    /// Completion of the most recent SaiLine move command or native live-stroke push.
    last_interactive_update: Option<Instant>,
}

/// `[p50, p95]` of `v` (nearest rank; zeros when empty).
fn percentiles(mut v: Vec<f64>) -> [f64; 2] {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return [0.0, 0.0];
    }
    v.sort_by(f64::total_cmp);
    let at = |p: f64| v.get(((p * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)).copied().unwrap_or(0.0);
    let r = |x: f64| (x * 100.0).round() / 100.0;
    [r(at(0.5)), r(at(0.95))]
}

impl StrokeTrace {
    /// Reads `PHOTOCRAFT_STROKE_TRACE` (off on the web, where there is no clock).
    pub fn from_env() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(v) = std::env::var_os("PHOTOCRAFT_STROKE_TRACE").filter(|v| !v.is_empty()) {
            let output = if v == "1" || v == "stderr" { Output::Stderr } else { Output::Csv(v.into()) };
            return Self { output, ..Default::default() };
        }
        Self::default()
    }

    pub fn enabled(&self) -> bool {
        self.output != Output::Off
    }

    /// A frame starts (the raw input hook) with `moves` pointer moves.
    pub fn begin_frame(&mut self, now: f64, moves: usize) {
        if !self.enabled() {
            return;
        }
        let interval = self.last_start.map_or(0.0, |t| now - t);
        self.last_start = Some(now);
        self.frame = Some(Row { t: now, interval, moves, ..Default::default() });
    }

    /// Timestamp the newest pointer-position event as eframe delivers it. This excludes OS/compositor queueing.
    pub fn pointer_event(&mut self) {
        if self.enabled() {
            self.last_pointer_received = Some(Instant::now());
        }
    }

    pub fn pointer_age_ms(&self) -> Option<f64> {
        if !self.enabled() {
            return None;
        }
        self.last_pointer_received.map(|at| at.elapsed().as_secs_f64() * 1000.0)
    }

    pub fn raw_hook_ms(&mut self, ms: f64) {
        if let Some(f) = self.frame.as_mut() { f.raw_hook = ms; }
    }

    pub fn logic_ms(&mut self, ms: f64) {
        if let Some(f) = self.frame.as_mut() { f.logic = ms; }
    }

    /// eframe reports a completed frame at the next app logic call. Attach it to the row that
    /// ended in the preceding App::ui call; it excludes measured vsync wait on wgpu backends.
    pub fn completed_frame_cpu(&mut self, secs: Option<f32>) {
        let Some(row) = self.rows.last_mut() else { return };
        let Some(cpu) = secs.map(|s| f64::from(s) * 1000.0) else { return };
        row.cpu_prev = Some(cpu);
        let tail = (cpu - row.raw_hook - row.logic - row.ui).max(0.0);
        row.cpu_after_ui = Some(tail);
        row.estimated_submit_age = Some(row.handoff_pointer_age + tail);
    }

    /// Test/offscreen hook for a frame CPU sample already paired with the current row.
    pub fn cpu_prev(&mut self, secs: Option<f32>) {
        if let Some(frame) = self.frame.as_mut() {
            frame.cpu_prev = secs.map(|s| f64::from(s) * 1000.0);
        }
    }

    /// Time spent rendering stroke points this frame.
    pub fn stroke(&mut self, ms: f64) {
        if let Some(f) = self.frame.as_mut() {
            f.stroke += ms;
        }
    }

    pub fn native_update(&mut self, input_age_ms: Option<f64>, render_ms: f64) {
        let Some(frame) = self.frame.as_mut() else { return };
        frame.native_input_age = input_age_ms.unwrap_or_default();
        frame.native_updates += 1;
        frame.stroke += render_ms;
        self.last_interactive_update = Some(Instant::now());
    }

    pub fn sailine_update(
        &mut self,
        input_ms: Option<f64>,
        command_ms: f64,
        pointer_age_ms: Option<f64>,
        stages: Option<photocraft_engine::linework_cmds::LineworkRefreshTimings>,
    ) {
        let Some(frame) = self.frame.as_mut() else { return };
        frame.sailine_events += 1;
        frame.sailine_input += input_ms.unwrap_or_default();
        frame.sailine_pointer_age = pointer_age_ms.unwrap_or_default();
        if command_ms > 0.0 {
            frame.sailine_updates += 1;
            frame.sailine_command += command_ms;
            self.last_interactive_update = Some(Instant::now());
        }
        if let Some(stages) = stages {
            frame.sailine_snapshot_clone += stages.snapshot_clone_ms;
            frame.sailine_shape += stages.shape_build_ms;
            frame.sailine_raster += stages.raster_ms;
            frame.sailine_path_cache += stages.path_cache_ms;
            frame.sailine_damage_bounds += stages.damage_bounds_ms;
            frame.sailine_composition += stages.composition_ms;
            frame.sailine_writeback += stages.writeback_ms;
            frame.sailine_refresh += stages.refresh_total_ms;
        }
    }

    /// Called by the main canvas after its GPU/CPU texture is current for this frame. This is
    /// the app-visible image preparation point, before eframe tessellation and backend submission.
    pub fn canvas_ready(&mut self) {
        let age = self.pointer_age_ms().unwrap_or_default();
        if let Some(frame) = self.frame.as_mut() {
            frame.canvas_ready_pointer_age = age;
            if let Some(updated) = self.last_interactive_update.take() {
                frame.canvas_ready_ms = updated.elapsed().as_secs_f64() * 1000.0;
            }
        }
    }

    /// Time spent refreshing the canvas this frame.
    pub fn canvas(&mut self, ms: f64) {
        if let Some(f) = self.frame.as_mut() {
            f.canvas += ms;
        }
    }

    pub fn canvas_details(&mut self, composite_ms: f64, upload_ms: f64, refreshes: u64, pixels: u64, gpu_uploads: u64) {
        if let Some(frame) = self.frame.as_mut() {
            frame.canvas_composite = composite_ms;
            frame.canvas_upload = upload_ms;
            frame.refreshes = refreshes;
            frame.refreshed_pixels = pixels;
            frame.gpu_uploads = gpu_uploads;
        }
    }

    /// The frame's UI is built (`ui` ms). `stroking`: a stroke is being drawn (a frame that
    /// rendered stroke points counts too, such as the release's commit). A stroke's frames are
    /// kept until the first frame without one, which ends it.
    pub fn end_frame(&mut self, ui: f64, stroking: bool, backend: impl FnOnce() -> String) {
        let Some(mut f) = self.frame.take() else { return };
        if f.sailine_events > 0 {
            f.sailine_pointer_age = self.pointer_age_ms().unwrap_or(f.sailine_pointer_age);
        }
        f.handoff_pointer_age = self.pointer_age_ms().unwrap_or_default();
        if stroking || f.stroke > 0.0 || f.sailine_events > 0 {
            f.ui = ui;
            // The first frame of a stroke doesn't follow a stroke frame: its interval is idle time.
            if self.rows.is_empty() {
                f.interval = 0.0;
            }
            self.rows.push(f);
        } else if !self.rows.is_empty() {
            self.finish(backend());
        }
    }

    fn finish(&mut self, backend: String) {
        let rows = std::mem::take(&mut self.rows);
        self.strokes += 1;
        let col = |g: fn(&Row) -> Option<f64>| percentiles(rows.iter().filter_map(g).collect());
        let summary = Summary {
            frames: rows.len(),
            backend,
            interval: col(|r| (r.interval > 0.0).then_some(r.interval)),
            fps: percentiles(rows.iter().filter_map(|r| (r.interval > 0.0).then_some(1000.0 / r.interval)).collect()),
            moves: col(|r| Some(r.moves as f64)),
            stroke: col(|r| Some(r.stroke)),
            native_input_age: col(|r| (r.native_updates > 0).then_some(r.native_input_age)),
            native_updates: col(|r| Some(r.native_updates as f64)),
            sailine_input: col(|r| (r.sailine_events > 0).then_some(r.sailine_input)),
            sailine_command: col(|r| (r.sailine_updates > 0).then_some(r.sailine_command)),
            sailine_pointer_age: col(|r| (r.sailine_events > 0).then_some(r.sailine_pointer_age)),
            sailine_snapshot_clone: col(|r| (r.sailine_updates > 0).then_some(r.sailine_snapshot_clone)),
            sailine_shape: col(|r| (r.sailine_updates > 0).then_some(r.sailine_shape)),
            sailine_raster: col(|r| (r.sailine_updates > 0).then_some(r.sailine_raster)),
            sailine_path_cache: col(|r| (r.sailine_updates > 0).then_some(r.sailine_path_cache)),
            sailine_damage_bounds: col(|r| (r.sailine_updates > 0).then_some(r.sailine_damage_bounds)),
            sailine_composition: col(|r| (r.sailine_updates > 0).then_some(r.sailine_composition)),
            sailine_writeback: col(|r| (r.sailine_updates > 0).then_some(r.sailine_writeback)),
            sailine_refresh: col(|r| (r.sailine_updates > 0).then_some(r.sailine_refresh)),
            sailine_events: col(|r| Some(r.sailine_events as f64)),
            sailine_updates: col(|r| Some(r.sailine_updates as f64)),
            canvas_ready_ms: col(|r| (r.canvas_ready_pointer_age > 0.0).then_some(r.canvas_ready_ms)),
            canvas_ready_pointer_age: col(|r| (r.canvas_ready_pointer_age > 0.0).then_some(r.canvas_ready_pointer_age)),
            handoff_pointer_age: col(|r| (r.handoff_pointer_age > 0.0).then_some(r.handoff_pointer_age)),
            estimated_submit_age: percentiles(rows.iter().filter_map(|r| r.estimated_submit_age).collect()),
            cpu_after_ui: percentiles(rows.iter().filter_map(|r| r.cpu_after_ui).collect()),
            canvas: col(|r| Some(r.canvas)),
            canvas_composite: col(|r| Some(r.canvas_composite)),
            canvas_upload: col(|r| Some(r.canvas_upload)),
            refreshes: col(|r| Some(r.refreshes as f64)),
            refreshed_pixels: col(|r| Some(r.refreshed_pixels as f64)),
            gpu_uploads: col(|r| Some(r.gpu_uploads as f64)),
            ui: col(|r| Some(r.ui)),
            cpu: percentiles(rows.iter().filter_map(|r| r.cpu_prev).collect()),
            wait: percentiles(rows.iter().filter_map(|r| Some((r.interval - r.cpu_prev?).max(0.0))).collect()),
        };
        if self.enabled() {
            eprintln!("{}", summary.line());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Output::Csv(path) = &self.output
            && let Err(e) = append_csv(path, self.strokes, &rows)
        {
            eprintln!("stroke trace: can't write {}: {e}", path.display());
        }
        self.last = Some(summary);
    }
}

/// The CSV text for `rows` of stroke `stroke` (with the header when `header`).
// The web build has no clock, so it never traces.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn csv(stroke: usize, rows: &[Row], header: bool) -> String {
    let mut s = String::new();
    if header {
        s.push_str("stroke,t_ms,interval_ms,moves,raw_hook_ms,logic_ms,brush_input_age_ms,brush_updates,stroke_ms,sai_events,sai_updates,sai_input_ms,sai_command_ms,sai_pointer_age_ms,sai_snapshot_clone_ms,sai_shape_ms,sai_raster_ms,sai_path_cache_ms,sai_damage_bounds_ms,sai_composition_ms,sai_writeback_ms,sai_refresh_ms,canvas_ready_ms,canvas_ready_pointer_age_ms,ui_handoff_pointer_age_ms,submit_age_estimate_ms,cpu_after_ui_estimate_ms,canvas_ms,canvas_composite_ms,texture_upload_ms,refreshes,refreshed_pixels,gpu_uploads,ui_ms,cpu_frame_ms\n");
    }
    for r in rows {
        let cpu = r.cpu_prev.map_or(String::new(), |c| format!("{c:.3}"));
        let after_ui = r.cpu_after_ui.map_or(String::new(), |v| format!("{v:.3}"));
        let submit_age = r.estimated_submit_age.map_or(String::new(), |v| format!("{v:.3}"));
        let _ = writeln!(s, "{stroke},{:.3},{:.3},{},{:.3},{:.3},{:.3},{},{:.3},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{submit_age},{after_ui},{:.3},{:.3},{:.3},{},{},{},{:.3},{cpu}",
            r.t, r.interval, r.moves, r.raw_hook, r.logic, r.native_input_age, r.native_updates, r.stroke,
            r.sailine_events, r.sailine_updates, r.sailine_input, r.sailine_command,
            r.sailine_pointer_age, r.sailine_snapshot_clone, r.sailine_shape, r.sailine_raster, r.sailine_path_cache, r.sailine_damage_bounds,
            r.sailine_composition, r.sailine_writeback, r.sailine_refresh, r.canvas_ready_ms, r.canvas_ready_pointer_age,
            r.handoff_pointer_age, r.canvas, r.canvas_composite, r.canvas_upload, r.refreshes, r.refreshed_pixels, r.gpu_uploads, r.ui);
    }
    s
}

#[cfg(not(target_arch = "wasm32"))]
fn append_csv(path: &std::path::Path, stroke: usize, rows: &[Row]) -> std::io::Result<()> {
    use std::io::Write as _;
    let header = std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(csv(stroke, rows, header).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> StrokeTrace {
        StrokeTrace { output: Output::Stderr, ..Default::default() }
    }

    fn frame(t: &mut StrokeTrace, now: f64, cpu: f32, stroking: bool) {
        t.begin_frame(now, 3);
        t.completed_frame_cpu(Some(cpu / 1000.0));
        t.cpu_prev(Some(cpu / 1000.0));
        if stroking {
            t.stroke(1.0);
            t.canvas(2.0);
        }
        t.end_frame(4.0, stroking, || "vulkan Test GPU".into());
    }

    #[test]
    fn a_stroke_is_its_frames_from_the_first_stroke_frame_to_the_first_without() {
        let mut t = on();
        frame(&mut t, 0.0, 5.0, false);
        frame(&mut t, 100.0, 5.0, true);
        for k in 1..=10 {
            frame(&mut t, 100.0 + 16.0 * f64::from(k), 6.0, true);
        }
        assert!(t.last.is_none(), "still drawing");
        frame(&mut t, 276.0, 6.0, false);
        let s = t.last.clone().expect("the stroke ended");
        assert_eq!(s.frames, 11);
        assert_eq!(s.interval, [16.0, 16.0], "the idle gap before the stroke isn't a frame interval");
        assert_eq!((s.stroke, s.canvas, s.ui, s.moves), ([1.0, 1.0], [2.0, 2.0], [4.0, 4.0], [3.0, 3.0]));
        assert_eq!((s.cpu, s.wait), ([6.0, 6.0], [10.0, 10.0]));
        assert!(s.line().contains("11 frames on vulkan Test GPU") && s.line().contains("wait 10.00/10.00"), "{}", s.line());
        // Off: nothing is kept.
        let mut off = StrokeTrace::default();
        frame(&mut off, 0.0, 1.0, true);
        frame(&mut off, 16.0, 1.0, false);
        assert!(off.last.is_none() && off.rows.is_empty());
    }

    #[test]
    fn csv_rows_and_odd_numbers() {
        let rows = [Row { t: 1.0, interval: 16.5, moves: 2, stroke: 0.25, canvas: 1.0, ui: 3.0, cpu_prev: None, ..Default::default() }];
        assert!(csv(1, &rows, true).starts_with("stroke,t_ms,interval_ms,moves,raw_hook_ms,logic_ms,brush_input_age_ms"));
        assert!(csv(1, &rows, false).starts_with("1,1.000,16.500,2,0.000,0.000,0.000,0,0.250,0,0,"));
        assert_eq!(percentiles(vec![]), [0.0, 0.0]);
        assert_eq!(percentiles(vec![f64::NAN, 2.0, f64::INFINITY]), [2.0, 2.0]);
        assert_eq!(percentiles((1..=100).map(f64::from).collect()), [51.0, 95.0]);
    }

    #[test]
    fn linework_drag_trace_keeps_command_stages_and_closes_after_release() {
        let mut t = on();
        t.begin_frame(0.0, 2);
        t.sailine_update(
            Some(0.4),
            0.8,
            Some(1.2),
            Some(photocraft_engine::linework_cmds::LineworkRefreshTimings {
                snapshot_clone_ms: 0.1,
                shape_build_ms: 0.2,
                raster_ms: 0.3,
                path_cache_ms: 0.05,
                damage_bounds_ms: 0.02,
                composition_ms: 0.06,
                writeback_ms: 0.01,
                refresh_total_ms: 0.72,
            }),
        );
        t.end_frame(2.0, false, || "test".into());
        t.begin_frame(16.0, 0);
        t.end_frame(1.0, false, || "test".into());
        let summary = t.last.expect("first idle frame closes the interaction trace");
        assert_eq!(summary.sailine_events, [1.0, 1.0]);
        assert_eq!(summary.sailine_updates, [1.0, 1.0]);
        assert_eq!(summary.sailine_input, [0.4, 0.4]);
        assert_eq!(summary.sailine_command, [0.8, 0.8]);
        assert_eq!(summary.sailine_shape, [0.2, 0.2]);
        assert_eq!(summary.sailine_raster, [0.3, 0.3]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_csv_gets_one_header_and_every_strokes_frames() {
        let dir = std::env::temp_dir().join(format!("photocraft-stroke-trace-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("trace.csv");
        let _ = std::fs::remove_file(&path);
        let mut t = StrokeTrace { output: Output::Csv(path.clone()), ..Default::default() };
        for start in [0.0, 1000.0] {
            for k in 0..3 {
                frame(&mut t, start + 16.0 * f64::from(k), 5.0, true);
            }
            frame(&mut t, start + 48.0, 5.0, false);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 7, "{text}");
        assert!(lines[0].starts_with("stroke,") && lines[1].starts_with("1,") && lines[6].starts_with("2,"));
    }
}
