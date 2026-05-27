//! Per-frame bump-allocated GPU buffer pool.
//!
//! Most of the per-frame instance buffers in `runtime/render.rs` (quad
//! batches, gradient quads, squircle quads, background instances, text
//! instance vertices, etc.) used to call `device.create_buffer_init`
//! once each, allocating a fresh `wgpu::Buffer` that gets dropped and
//! deferred-destroyed at frame end. On the Steam Deck SD that adds up
//! to a measurable wgpu tracker cost plus extra Vulkan allocator work.
//!
//! [`FrameBufferPool`] replaces those per-frame buffer creates with one
//! growable persistent `wgpu::Buffer` (`VERTEX | COPY_DST`) per category.
//! Each `alloc<T>(...)` rounds the write cursor up to a 4-byte boundary,
//! `queue.write_buffer`s the data into the pool, and returns a
//! [`PoolSlice`] (offset + byte length). The cursor is reset every
//! [`FrameBufferPool::begin_frame`].
//!
//! Pools are intentionally per-purpose so callers can keep their flat
//! `Vec<PoolSlice>` indexed-by-`buf_idx` shape — switching ProcessOpCtx
//! to one shared pool would require collapsing all the slice arrays
//! together, which the existing op-dispatch code isn't structured for.
//!
//! Not all per-frame buffers are routed through the pool yet; see the
//! TODO in `runtime/render.rs` near `flame_buffers` /
//! `tile_face_inst_buffers` / `image_quad_inst_buffers` /
//! `tile_glow_buffer` / `relic_glow_buffer` / `relic_debuff_buffer`.
//! Those are smaller and rarer per-frame, so prioritising the high-
//! frequency offenders (quad batches and text label vertex instances)
//! per the performance review was the brief.

use wgpu::util::DeviceExt;

const MIN_INITIAL_CAPACITY: u64 = 64 * 1024;
const ALIGN: u64 = 4;

/// One growable persistent `wgpu::Buffer` carved up by per-frame bump
/// allocation. Reset with [`FrameBufferPool::begin_frame`] at the top
/// of every `WgpuRenderer::render` so the cursor falls back to 0 and
/// the previous frame's contents are overwritten in place.
pub(super) struct FrameBufferPool {
    buffer: wgpu::Buffer,
    capacity: u64,
    cursor: u64,
    label: &'static str,
}

/// A view into a [`FrameBufferPool`]'s underlying buffer. Callers turn
/// this back into a `wgpu::BufferSlice` at draw time via
/// [`FrameBufferPool::buffer_slice`].
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PoolSlice {
    pub offset: u64,
    pub byte_len: u64,
}

impl FrameBufferPool {
    pub(super) fn new(device: &wgpu::Device, label: &'static str, initial_capacity: u64) -> Self {
        let capacity = initial_capacity.max(MIN_INITIAL_CAPACITY);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            capacity,
            cursor: 0,
            label,
        }
    }

    /// Reset the bump cursor — call once at the top of `render()`.
    pub(super) fn begin_frame(&mut self) {
        self.cursor = 0;
    }

    /// Borrow the underlying buffer for `pass.set_vertex_buffer(...)`.
    pub(super) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Bump-allocate `data.len() * sizeof(T)` bytes and queue a buffer
    /// upload. Returns the `(offset, byte_len)` for use at draw time.
    /// Grows the underlying buffer (re-create + queue.write_buffer
    /// works because no command encoder has yet referenced the slice
    /// returned this frame: we always queue uploads via `queue` and
    /// the slice is read in a later pass on the same encoder).
    pub(super) fn alloc<T: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[T],
    ) -> PoolSlice {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let byte_len = bytes.len() as u64;
        if byte_len == 0 {
            return PoolSlice {
                offset: self.cursor,
                byte_len: 0,
            };
        }
        // Round cursor up to vertex-buffer alignment (4 bytes).
        let aligned = (self.cursor + ALIGN - 1) & !(ALIGN - 1);
        let needed = aligned + byte_len;
        if needed > self.capacity {
            // Grow geometrically; re-create the underlying buffer.
            // Any prior slices for this frame became invalid the
            // moment this returns (they pointed into the old buffer
            // ID), but we always grow at the *moment of allocation*
            // before the encoder references the new slice — and the
            // alternative is to crash. Calling sites that keep slices
            // across reallocs would be incorrect; current call sites
            // only use slices in the same render pass, after all
            // allocations for that op have happened.
            let mut new_capacity = self.capacity.saturating_mul(2).max(needed);
            // Round up to a 4 KiB page for friendlier allocator
            // behavior.
            new_capacity = (new_capacity + 4095) & !4095;
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(self.label),
                contents: &vec![0u8; new_capacity as usize],
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.buffer = new_buffer;
            self.capacity = new_capacity;
        }
        queue.write_buffer(&self.buffer, aligned, bytes);
        self.cursor = aligned + byte_len;
        PoolSlice {
            offset: aligned,
            byte_len,
        }
    }
}
