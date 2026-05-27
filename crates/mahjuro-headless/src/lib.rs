//! Offscreen GPU tools: room lighting bakes and marketing screenshots.
//!
//! Built as a separate workspace crate so `cargo build -p mahjuro-bake` does not produce the
//! interactive game binary. Bake and screenshot share [`HeadlessApp`] but use different feature
//! flags on the `mahjuro` dependency: bake uses `bake-support` (no SDL/Steam/rodio);
//! screenshot pulls in bot + draw helpers via `headless-screenshot`.

mod app;
mod fixtures;
mod room_bake_scenes;
mod slug;

pub mod bake_cli;
pub mod screenshot_cli;

#[cfg(feature = "bake")]
mod bake;
#[cfg(feature = "screenshot")]
mod screenshot_scenes;
#[cfg(feature = "screenshot")]
mod screenshot;

#[cfg(feature = "bake")]
pub use bake::run as run_bake;
fn init_env_logger() {
    use std::fs::OpenOptions;
    use std::io::LineWriter;
    use std::path::Path;

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(path_raw) = std::env::var_os("MAHJURO_LOG_FILE") {
        let path = Path::new(&path_raw);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
            builder.target(env_logger::Target::Pipe(Box::new(LineWriter::new(f))));
        }
    }
    let _ = builder.try_init();
}

#[cfg(feature = "bake")]
pub fn run_bake_from_argv() -> anyhow::Result<()> {
    use clap::Parser;
    init_env_logger();
    let cli = bake_cli::BakeRoomCli::parse();
    bake::run(cli)
}

#[cfg(feature = "screenshot")]
pub fn run_screenshot_from_argv() -> anyhow::Result<()> {
    use clap::Parser;
    init_env_logger();
    let cli = screenshot_cli::ScreenshotCli::parse();
    screenshot::run(cli)
}

/// Run a capture from a parsed CLI (for tests or programmatic use).
#[cfg(feature = "screenshot")]
pub fn run_screenshot(cli: screenshot_cli::ScreenshotCli) -> anyhow::Result<()> {
    screenshot::run(cli)
}
