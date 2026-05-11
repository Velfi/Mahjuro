use super::*;

mod build;

impl WgpuRenderer {
    pub fn new(target_init: TargetInit) -> anyhow::Result<Self> {
        build::build_renderer_new(target_init)
    }
}
