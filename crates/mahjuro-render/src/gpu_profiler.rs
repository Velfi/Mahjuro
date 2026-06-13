//! Optional GPU pass profiler built on `wgpu::Features::TIMESTAMP_QUERY`.
//!
//! Activated on demand from the Debug menu. While a profile session is
//! active the renderer wraps each major render pass in
//! `RenderPassTimestampWrites` (or compute-pass equivalents) so the GPU
//! records start/end timestamps into a shared `QuerySet`. After every
//! frame's submit we resolve the queries into a CPU-readable buffer,
//! accumulate per-pass durations, and once the requested frame count has
//! been reached we log the averages.
//!
//! Sessions block on `device.poll(Wait)` between frames so the readback is
//! synchronous and frame-accurate at the cost of throughput — fine for a
//! one-shot debug capture.
//!
//! Pass slots (see [`PassSlot`]):
//!   shadow, main, main-table, main-scene — Pass A (table split only while profiling)
//!   cascade — shooting-star offscreen pre-pass
//!   room-bloom — second GLB draw for linear HDR bloom / emissive (shop, hallway, archive)
//!   bloom-extract, bloom-blur-h, bloom-blur-v, scene-composite
//!   tonemap, overlay — final display + 2D HUD text
//!
//! On Metal / Apple Silicon, per-pass begin/end pairs in one encoder are unreliable
//! (later passes report cumulative GPU time). Those backends use **chained fences**:
//! one timestamp at the end of each pass, duration = `fence[i+1] - fence[i]`.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

const NUM_PASSES: usize = 12;
/// Pairwise begin/end indices (Vulkan / DX12).
const PAIRWISE_TIMESTAMPS: u32 = (NUM_PASSES * 2) as u32;
const QUERY_COUNT: u32 = PAIRWISE_TIMESTAMPS; // 32 slots; Metal chained mode uses ≤17
const TIMESTAMP_BYTES: u64 = 8;
const BUFFER_SIZE: u64 = QUERY_COUNT as u64 * TIMESTAMP_BYTES;

const PASS_LABELS: [&str; NUM_PASSES] = [
    "shadow",
    "main",
    "main-table",
    "main-scene",
    "cascade",
    "room-bloom",
    "bloom-extract",
    "bloom-blur-h",
    "bloom-blur-v",
    "scene-composite",
    "tonemap",
    "overlay",
];

/// Per-pass timestamp slot indices into the shared query set.
#[derive(Copy, Clone, Debug)]
#[repr(usize)]
pub enum PassSlot {
    Shadow = 0,
    /// Pass A when not split for profiling.
    Main = 1,
    /// Pass A — table mesh. Mutually exclusive with [`PassSlot::Main`] when
    /// Pass A is split during a GPU profile session.
    MainTable = 2,
    /// Pass A — everything in Pass A except the table draw.
    MainScene = 3,
    Cascade = 4,
    /// `shop-linear-bloom-pass` / hallway / archive linear HDR room redraw.
    RoomBloom = 5,
    BloomExtract = 6,
    BloomBlurH = 7,
    BloomBlurV = 8,
    SceneComposite = 9,
    Tonemap = 10,
    Overlay = 11,
}

pub struct GpuProfiler {
    /// True only when the adapter advertised TIMESTAMP_QUERY and the
    /// renderer successfully created a query set.
    enabled: bool,
    /// Nanoseconds per timestamp tick (from `Queue::get_timestamp_period`).
    period_ns: f32,

    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: Option<wgpu::Buffer>,
    readback_buffer: Option<wgpu::Buffer>,

    /// True between `start()` and the final frame of a session.
    sampling: bool,
    frames_remaining: u32,
    total_frames: u32,
    /// Swapchain resolution at capture start, logged in the report.
    capture_width: u32,
    capture_height: u32,
    /// Per-pass accumulated GPU time in milliseconds.
    accum_ms: [f64; NUM_PASSES],
    /// Number of frames each pass actually ran during the session (some
    /// passes are conditional, e.g. main-table only when Pass A is split).
    pass_frame_counts: [u32; NUM_PASSES],
    /// Which passes ran in the most recent frame, so the readback knows
    /// which slot pairs to trust. Written via `Cell` from `pass_writes`
    /// because callers hold an immutable borrow of `self` (closures
    /// capturing `&self` for other fields) while encoding the frame.
    last_frame_passes: [Cell<bool>; NUM_PASSES],
    /// When false, [`Self::pass_writes`] / submit readback are skipped for
    /// this `render_to` call (e.g. shop journal pre-pass into an offscreen
    /// target). Only the swapchain submission advances the capture.
    primary_submit: Cell<bool>,
    /// Metal: chained end timestamps; other backends: per-pass begin/end pairs.
    chained_fences: bool,
    /// Chained mode: number of instrumented passes this submit (reset in [`Self::begin_submit`]).
    frame_fence_count: Cell<u32>,
    /// Chained mode: pass slot per fence, in submission order.
    frame_fence_slots: Cell<[u8; NUM_PASSES]>,
    /// Latched on the frame the session ends (after `report()`). Polled by
    /// the app once per frame via [`Self::take_just_completed`] to play a
    /// confirmation SFX, so the player knows the capture is done without
    /// watching the log stream.
    just_completed: bool,
}

impl GpuProfiler {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        supported: bool,
        backend: wgpu::Backend,
    ) -> Self {
        if !supported {
            return Self::disabled();
        }
        let chained_fences = backend == wgpu::Backend::Metal;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("gpu-profiler-timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu-profiler-resolve"),
            size: BUFFER_SIZE,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu-profiler-readback"),
            size: BUFFER_SIZE,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            enabled: true,
            period_ns: queue.get_timestamp_period(),
            query_set: Some(query_set),
            resolve_buffer: Some(resolve_buffer),
            readback_buffer: Some(readback_buffer),
            sampling: false,
            frames_remaining: 0,
            total_frames: 0,
            capture_width: 0,
            capture_height: 0,
            accum_ms: [0.0; NUM_PASSES],
            pass_frame_counts: [0; NUM_PASSES],
            last_frame_passes: [const { Cell::new(false) }; NUM_PASSES],
            primary_submit: Cell::new(true),
            chained_fences,
            frame_fence_count: Cell::new(0),
            frame_fence_slots: Cell::new([0u8; NUM_PASSES]),
            just_completed: false,
        }
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            period_ns: 0.0,
            query_set: None,
            resolve_buffer: None,
            readback_buffer: None,
            sampling: false,
            frames_remaining: 0,
            total_frames: 0,
            capture_width: 0,
            capture_height: 0,
            accum_ms: [0.0; NUM_PASSES],
            pass_frame_counts: [0; NUM_PASSES],
            last_frame_passes: [const { Cell::new(false) }; NUM_PASSES],
            primary_submit: Cell::new(true),
            chained_fences: false,
            frame_fence_count: Cell::new(0),
            frame_fence_slots: Cell::new([0u8; NUM_PASSES]),
            just_completed: false,
        }
    }

    /// Consume the "session just ended" latch. Returns `true` exactly once,
    /// on the frame after [`Self::report`] fires.
    pub fn take_just_completed(&mut self) -> bool {
        std::mem::take(&mut self.just_completed)
    }

    /// Begin sampling for the next `frames` rendered frames. No-op (with a
    /// warning) when the device doesn't support timestamp queries or when a
    /// session is already in flight.
    pub fn start(&mut self, frames: u32, width: u32, height: u32) {
        if !self.enabled {
            log::warn!("TIMESTAMP_QUERY not supported by this adapter; cannot profile");
            return;
        }
        if self.sampling {
            log::warn!("GPU profile already running; ignoring start request");
            return;
        }
        let frames = frames.max(1);
        self.sampling = true;
        self.frames_remaining = frames;
        self.total_frames = frames;
        self.capture_width = width;
        self.capture_height = height;
        self.accum_ms = [0.0; NUM_PASSES];
        self.pass_frame_counts = [0; NUM_PASSES];
        for c in &self.last_frame_passes {
            c.set(false);
        }
        log::debug!("Starting GPU profile capture over {frames} frames");
    }

    /// Mark whether this `render_to` submission owns the active capture
    /// (swapchain) or is a secondary target (journal pre-pass).
    pub fn begin_submit(&self, primary: bool) {
        self.primary_submit.set(primary);
        if primary {
            self.frame_fence_count.set(0);
        }
    }

    fn profile_this_submit(&self) -> bool {
        self.sampling && self.primary_submit.get()
    }

    /// Build the timestamp_writes descriptor for a render pass slot. Returns
    /// `None` when no session is active so callers can pass it straight into
    /// `RenderPassDescriptor`. Records that the pass ran this frame.
    pub fn pass_writes(&self, slot: PassSlot) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.profile_this_submit() {
            return None;
        }
        let qs = self.query_set.as_ref()?;
        let idx = slot as usize;
        self.last_frame_passes[idx].set(true);
        if self.chained_fences {
            let f = self.frame_fence_count.get();
            if f as usize >= NUM_PASSES {
                log::warn!(
                    "GPU profiler: too many passes in one submit; dropping timestamp for {slot:?}"
                );
                return None;
            }
            let mut slots = self.frame_fence_slots.get();
            slots[f as usize] = idx as u8;
            self.frame_fence_slots.set(slots);
            self.frame_fence_count.set(f + 1);
            let end_idx = f + 1;
            let begin_idx = if f == 0 { Some(0) } else { None };
            Some(wgpu::RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: begin_idx,
                end_of_pass_write_index: Some(end_idx),
            })
        } else {
            let begin = (idx * 2) as u32;
            Some(wgpu::RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some(begin),
                end_of_pass_write_index: Some(begin + 1),
            })
        }
    }

    /// Same as [`Self::pass_writes`] for compute passes.
    pub fn compute_pass_writes(
        &self,
        slot: PassSlot,
    ) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        if !self.profile_this_submit() {
            return None;
        }
        let qs = self.query_set.as_ref()?;
        let idx = slot as usize;
        self.last_frame_passes[idx].set(true);
        if self.chained_fences {
            let f = self.frame_fence_count.get();
            if f as usize >= NUM_PASSES {
                log::warn!(
                    "GPU profiler: too many passes in one submit; dropping timestamp for {slot:?}"
                );
                return None;
            }
            let mut slots = self.frame_fence_slots.get();
            slots[f as usize] = idx as u8;
            self.frame_fence_slots.set(slots);
            self.frame_fence_count.set(f + 1);
            let end_idx = f + 1;
            let begin_idx = if f == 0 { Some(0) } else { None };
            Some(wgpu::ComputePassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: begin_idx,
                end_of_pass_write_index: Some(end_idx),
            })
        } else {
            let begin = (idx * 2) as u32;
            Some(wgpu::ComputePassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some(begin),
                end_of_pass_write_index: Some(begin + 1),
            })
        }
    }

    /// Called once per frame after all passes have been encoded but before
    /// `queue.submit`. Resolves the query set and stages a copy into the
    /// CPU-readable buffer.
    pub fn before_submit(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.profile_this_submit() {
            return;
        }
        let (Some(qs), Some(resolve), Some(readback)) = (
            self.query_set.as_ref(),
            self.resolve_buffer.as_ref(),
            self.readback_buffer.as_ref(),
        ) else {
            return;
        };
        let resolve_end = if self.chained_fences {
            self.frame_fence_count.get() + 1
        } else {
            PAIRWISE_TIMESTAMPS
        };
        encoder.resolve_query_set(qs, 0..resolve_end, resolve, 0);
        encoder.copy_buffer_to_buffer(resolve, 0, readback, 0, BUFFER_SIZE);
    }

    /// Called once per frame after `queue.submit`. Polls the device, maps
    /// the readback buffer, accumulates per-pass timings, and logs the
    /// averages on the final frame.
    pub fn after_submit(&mut self, device: &wgpu::Device) {
        if !self.profile_this_submit() {
            return;
        }
        let Some(readback) = self.readback_buffer.as_ref() else {
            return;
        };

        // Set up a one-shot map. We use an Arc<Mutex<Option<Result>>> rather
        // than a channel to avoid pulling in extra deps.
        let map_result: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>> =
            Arc::new(Mutex::new(None));
        let map_result_cb = Arc::clone(&map_result);
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            *map_result_cb.lock().unwrap() = Some(r);
        });
        // Block until the GPU has finished and the map callback has fired.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let map_outcome = map_result.lock().unwrap().take();
        match map_outcome {
            Some(Ok(())) => {
                let view = readback.slice(..).get_mapped_range();
                let raw: &[u64] = bytemuck::cast_slice(&view);
                if self.chained_fences {
                    let n = self.frame_fence_count.get() as usize;
                    for f in 0..n {
                        let begin = raw[f];
                        let end = raw[f + 1];
                        if end <= begin || begin == 0 || end == 0 {
                            continue;
                        }
                        let i = self.frame_fence_slots.get()[f] as usize;
                        if i >= NUM_PASSES {
                            continue;
                        }
                        let ticks = end - begin;
                        let ns = ticks as f64 * self.period_ns as f64;
                        self.accum_ms[i] += ns / 1.0e6;
                        self.pass_frame_counts[i] += 1;
                    }
                } else {
                    debug_assert_eq!(raw.len(), PAIRWISE_TIMESTAMPS as usize);
                    for (i, _label) in PASS_LABELS.iter().enumerate() {
                        if !self.last_frame_passes[i].get() {
                            continue;
                        }
                        let begin = raw[i * 2];
                        let end = raw[i * 2 + 1];
                        if end <= begin || begin == 0 || end == 0 {
                            continue;
                        }
                        let ticks = end - begin;
                        let ns = ticks as f64 * self.period_ns as f64;
                        self.accum_ms[i] += ns / 1.0e6;
                        self.pass_frame_counts[i] += 1;
                    }
                }
                drop(view);
                readback.unmap();
            }
            Some(Err(err)) => {
                log::error!("[GpuProfiler] readback map failed: {err:?}");
            }
            None => {
                log::error!("[GpuProfiler] readback map callback never fired");
            }
        }

        // Reset per-frame pass tracking for the next frame.
        for c in &self.last_frame_passes {
            c.set(false);
        }

        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        if self.frames_remaining == 0 {
            self.report();
            self.sampling = false;
            self.just_completed = true;
        }
    }

    fn report(&self) {
        let mut acc = String::new();
        acc.push_str(&format!(
            "=== GPU pass timings averaged over {} frames ({}×{})",
            self.total_frames, self.capture_width, self.capture_height,
        ));
        if self.chained_fences {
            acc.push_str(" [Metal chained fences]");
        }
        acc.push('\n');
        let mut total = 0.0_f64;
        for (i, label) in PASS_LABELS.iter().enumerate() {
            let frames = self.pass_frame_counts[i];
            if frames == 0 {
                acc.push_str(&format!("   {label:<16} (not run)\n"));
                continue;
            }
            let avg = self.accum_ms[i] / frames as f64;
            total += avg;
            acc.push_str(&format!(
                "   {label:<16} {avg:>7.3} ms  ({frames} frames)\n"
            ));
        }
        acc.push_str(&format!(
            "   {:<16} {total:>7.3} ms (sum of averages)\n",
            "TOTAL"
        ));
        log::debug!("{}", acc);
    }
}
