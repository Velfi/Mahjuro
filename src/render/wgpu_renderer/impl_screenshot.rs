use super::*;

impl WgpuRenderer {
    fn encode_screenshot_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        tex: &wgpu::Texture,
        _path: &std::path::Path,
    ) -> ScreenshotStaging {
        let width = tex.width();
        let height = tex.height();
        // wgpu requires bytes_per_row to be a multiple of 256.
        let bytes_per_pixel: u32 = 4; // BGRA8 / RGBA8 — 4 bytes
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
        let buffer_size = (padded_bytes_per_row as u64) * (height as u64);

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot-staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        ScreenshotStaging {
            buffer,
            width,
            height,
            padded_bytes_per_row,
            unpadded_bytes_per_row,
            format: tex.format(),
        }
    }

    /// Map the staging buffer, decode pixels (handling BGRA→RGBA + row
    /// stride), and write the PNG. Synchronous: blocks on `device.poll`.
    fn finalize_screenshot(
        &self,
        staging: ScreenshotStaging,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let slice = staging.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // Block until the GPU finishes the copy.
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv()??;

        let data = slice.get_mapped_range();

        // Strip row padding and (if needed) swap BGRA → RGBA.
        let w = staging.width as usize;
        let h = staging.height as usize;
        let unpadded = staging.unpadded_bytes_per_row as usize;
        let padded = staging.padded_bytes_per_row as usize;
        let mut pixels: Vec<u8> = Vec::with_capacity(w * h * 4);
        let swap_bgra = matches!(
            staging.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        for row in 0..h {
            let row_start = row * padded;
            let row_end = row_start + unpadded;
            let row_pixels = &data[row_start..row_end];
            if swap_bgra {
                for chunk in row_pixels.chunks_exact(4) {
                    pixels.push(chunk[2]);
                    pixels.push(chunk[1]);
                    pixels.push(chunk[0]);
                    pixels.push(chunk[3]);
                }
            } else {
                pixels.extend_from_slice(row_pixels);
            }
        }
        drop(data);
        staging.buffer.unmap();

        let img = image::RgbaImage::from_raw(staging.width, staging.height, pixels)
            .ok_or_else(|| anyhow::anyhow!("RgbaImage::from_raw failed"))?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        img.save(path)?;
        Ok(())
    }
}
