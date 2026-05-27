/// Per-screenshot staging buffer + the metadata needed to decode it.
pub(crate) struct ScreenshotStaging {
    pub buffer: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    pub padded_bytes_per_row: u32,
    pub unpadded_bytes_per_row: u32,
    pub format: wgpu::TextureFormat,
}
