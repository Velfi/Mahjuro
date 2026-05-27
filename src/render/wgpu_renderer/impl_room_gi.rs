use super::*;

pub(crate) struct RoomGiCaptureStaging {
    buffer: wgpu::Buffer,
    byte_len: u64,
    meta: crate::render::room_gi_bake::RoomGiBake,
}

impl WgpuRenderer {
    /// Queue a one-shot GPU readback of probe SH after the next dynamic GI compute
    /// (`mahjuro bake-room`).
    pub fn request_room_gi_capture(&mut self, room: crate::render::room_gi_bake::RoomGiRoom) {
        self.room_gi_capture_pending = Some(room);
        self.room_gi_captured = None;
        self.room_gi_capture_meta = None;
    }

    pub fn take_room_gi_capture(&mut self) -> Option<crate::render::room_gi_bake::RoomGiBake> {
        self.room_gi_captured.take()
    }

    pub(crate) fn encode_room_gi_capture_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        meta: crate::render::room_gi_bake::RoomGiBake,
    ) -> RoomGiCaptureStaging {
        let byte_len = meta.probe_sh_bytes.len() as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-gi-capture-staging"),
            size: byte_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.probe_sh_buffer, 0, &staging, 0, byte_len);
        RoomGiCaptureStaging {
            buffer: staging,
            byte_len,
            meta,
        }
    }

    pub(crate) fn finalize_room_gi_capture(
        &self,
        staging: RoomGiCaptureStaging,
    ) -> anyhow::Result<crate::render::room_gi_bake::RoomGiBake> {
        let slice = staging.buffer.slice(..staging.byte_len);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| anyhow::anyhow!("room GI capture map channel closed"))?
            .map_err(|e| anyhow::anyhow!("room GI capture map failed: {e:?}"))?;
        let mapped = slice.get_mapped_range();
        let mut bake = staging.meta;
        let mut bytes = bake.probe_sh_bytes.to_vec();
        bytes.copy_from_slice(&mapped);
        bake.probe_sh_bytes = std::sync::Arc::from(bytes);
        drop(mapped);
        staging.buffer.unmap();
        Ok(bake)
    }
}
