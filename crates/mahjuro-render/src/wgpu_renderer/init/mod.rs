use super::*;

pub mod bloom;
mod build;
mod shaders_and_pipelines;

impl WgpuRenderer {
    pub fn new(target_init: TargetInit) -> anyhow::Result<Self> {
        #[cfg(feature = "windowed")]
        {
            build::build_renderer_new(target_init, false, None)
        }
        #[cfg(not(feature = "windowed"))]
        {
            build::build_renderer_new(target_init)
        }
    }

    /// Windowed game entry: optionally present a black boot frame as soon as the
    /// swapchain exists (Steam Deck / gamescope — see `present_boot_loading_clear`).
    #[cfg(feature = "windowed")]
    pub fn new_windowed(
        window: sdl3::video::Window,
        hdr_enabled: bool,
        present_boot_frame: bool,
        boot_input_poll: Option<&mut dyn FnMut()>,
    ) -> anyhow::Result<Self> {
        build::build_renderer_new(
            TargetInit::Windowed {
                window,
                hdr_enabled,
            },
            present_boot_frame,
            boot_input_poll,
        )
    }
}
