//! Optional GPU pass profiler built on `wgpu::Features::TIMESTAMP_QUERY`.
//!
//! Activated on demand from the Debug menu. While a profile session is
//! active the renderer wraps each major render pass in
//! `RenderPassTimestampWrites` so the GPU records start/end timestamps into
//! a shared `QuerySet`. After every frame's submit we resolve the queries
//! into a CPU-readable buffer, accumulate per-pass durations, and once the
//! requested frame count has been reached we log the averages.
//!
//! Sessions block on `device.poll(Wait)` between frames so the readback is
//! synchronous and frame-accurate at the cost of throughput — fine for a
//! one-shot debug capture.
//!
//! Pass slots:
//!   0/1  shadow pre-pass
//!   2/3  main pass (Pass A)
//!   4/5  post-smoke pass (Pass B, optional)

use std::sync::{Arc, Mutex};

const NUM_PASSES: usize = 4;
const NUM_TIMESTAMPS: u32 = (NUM_PASSES * 2) as u32;
const TIMESTAMP_BYTES: u64 = 8;
const BUFFER_SIZE: u64 = NUM_TIMESTAMPS as u64 * TIMESTAMP_BYTES;

const PASS_LABELS: [&str; NUM_PASSES] = ["shadow", "main", "smoke-offscreen", "post-smoke"];

/// Per-pass timestamp slot indices into the shared query set.
#[derive(Copy, Clone)]
pub enum PassSlot {
    Shadow = 0,
    Main = 1,
    SmokeOffscreen = 2,
    PostSmoke = 3,
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
    /// Per-pass accumulated GPU time in milliseconds.
    accum_ms: [f64; NUM_PASSES],
    /// Number of frames each pass actually ran during the session (some
    /// passes are conditional, e.g. post-smoke only fires when fluid smoke
    /// is on screen).
    pass_frame_counts: [u32; NUM_PASSES],
    /// Which passes ran in the most recent frame, so the readback knows
    /// which slot pairs to trust.
    last_frame_passes: [bool; NUM_PASSES],
}

impl GpuProfiler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, supported: bool) -> Self {
        if !supported {
            return Self::disabled();
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("gpu-profiler-timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: NUM_TIMESTAMPS,
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
            accum_ms: [0.0; NUM_PASSES],
            pass_frame_counts: [0; NUM_PASSES],
            last_frame_passes: [false; NUM_PASSES],
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
            accum_ms: [0.0; NUM_PASSES],
            pass_frame_counts: [0; NUM_PASSES],
            last_frame_passes: [false; NUM_PASSES],
        }
    }

    /// Begin sampling for the next `frames` rendered frames. No-op (with a
    /// warning) when the device doesn't support timestamp queries or when a
    /// session is already in flight.
    pub fn start(&mut self, frames: u32) {
        if !self.enabled {
            log::warn!(
                "[GpuProfiler] TIMESTAMP_QUERY not supported by this adapter; cannot profile"
            );
            return;
        }
        if self.sampling {
            log::warn!("[GpuProfiler] profile already running; ignoring start request");
            return;
        }
        let frames = frames.max(1);
        self.sampling = true;
        self.frames_remaining = frames;
        self.total_frames = frames;
        self.accum_ms = [0.0; NUM_PASSES];
        self.pass_frame_counts = [0; NUM_PASSES];
        self.last_frame_passes = [false; NUM_PASSES];
        log::info!("[GpuProfiler] starting capture over {frames} frames");
    }

    #[allow(dead_code)]
    pub fn is_sampling(&self) -> bool {
        self.sampling
    }

    /// Build the timestamp_writes descriptor for a render pass slot. Returns
    /// `None` when no session is active so callers can pass it straight into
    /// `RenderPassDescriptor`. Records that the pass ran this frame.
    pub fn pass_writes(&mut self, slot: PassSlot) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.sampling {
            return None;
        }
        let qs = self.query_set.as_ref()?;
        let idx = slot as usize;
        self.last_frame_passes[idx] = true;
        let begin = (idx * 2) as u32;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some(begin),
            end_of_pass_write_index: Some(begin + 1),
        })
    }

    /// Called once per frame after all passes have been encoded but before
    /// `queue.submit`. Resolves the query set and stages a copy into the
    /// CPU-readable buffer.
    pub fn before_submit(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.sampling {
            return;
        }
        let (Some(qs), Some(resolve), Some(readback)) = (
            self.query_set.as_ref(),
            self.resolve_buffer.as_ref(),
            self.readback_buffer.as_ref(),
        ) else {
            return;
        };
        encoder.resolve_query_set(qs, 0..NUM_TIMESTAMPS, resolve, 0);
        encoder.copy_buffer_to_buffer(resolve, 0, readback, 0, BUFFER_SIZE);
    }

    /// Called once per frame after `queue.submit`. Polls the device, maps
    /// the readback buffer, accumulates per-pass timings, and logs the
    /// averages on the final frame.
    pub fn after_submit(&mut self, device: &wgpu::Device) {
        if !self.sampling {
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
                debug_assert_eq!(raw.len(), NUM_PASSES * 2);
                for (i, label) in PASS_LABELS.iter().enumerate() {
                    if !self.last_frame_passes[i] {
                        continue;
                    }
                    let begin = raw[i * 2];
                    let end = raw[i * 2 + 1];
                    // Guard against wraparound or empty passes that wrote
                    // zeros (shouldn't happen, but cheap to check).
                    if end <= begin {
                        continue;
                    }
                    let ticks = end - begin;
                    let ns = ticks as f64 * self.period_ns as f64;
                    self.accum_ms[i] += ns / 1.0e6;
                    self.pass_frame_counts[i] += 1;
                    let _ = label;
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
        self.last_frame_passes = [false; NUM_PASSES];

        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        if self.frames_remaining == 0 {
            self.report();
            self.sampling = false;
        }
    }

    fn report(&self) {
        log::info!(
            "[GpuProfiler] === GPU pass timings averaged over {} frames ===",
            self.total_frames
        );
        let mut total = 0.0_f64;
        for (i, label) in PASS_LABELS.iter().enumerate() {
            let frames = self.pass_frame_counts[i];
            if frames == 0 {
                log::info!("[GpuProfiler]   {label:<12} (not run)");
                continue;
            }
            let avg = self.accum_ms[i] / frames as f64;
            total += avg;
            log::info!("[GpuProfiler]   {label:<12} {avg:>7.3} ms  ({frames} frames)");
        }
        log::info!(
            "[GpuProfiler]   {:<12} {total:>7.3} ms (sum of averages)",
            "TOTAL"
        );
    }
}
