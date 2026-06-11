#[cfg(feature = "windowed")]
use sdl3::video::Window;

/// Where the final composited frame lands. `Surface` is the normal
/// swapchain path used by the interactive game; `Offscreen` is a plain
/// render-attachment texture used by headless screenshot mode (no window,
/// no window-server occlusion games, no swapchain `Outdated` retries).
///
/// `config` on `WgpuRenderer` still holds the format/size that downstream
/// scene-color/post textures track against — the offscreen path writes the
/// same values there so `resize()` and the various post textures don't
/// need to branch.
pub(crate) enum RenderTarget {
    Surface(wgpu::Surface<'static>),
    Offscreen { texture: wgpu::Texture },
}

/// Where the renderer should send frames. Chosen once at construction:
/// the interactive game builds a `Windowed`; the screenshot CLI builds a
/// `Headless`.
pub enum TargetInit {
    #[cfg(feature = "windowed")]
    Windowed { window: Window, hdr_enabled: bool },
    Headless {
        width: u32,
        height: u32,
        hdr_enabled: bool,
    },
}
