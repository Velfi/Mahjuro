use std::path::PathBuf;

/// Return the root `assets/` directory.
///
/// - Debug builds: resolved from `CARGO_MANIFEST_DIR` (source tree).
/// - Release builds: resolved next to the running executable.
pub fn assets_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
    } else {
        std::env::current_exe()
            .expect("failed to get executable path")
            .parent()
            .expect("executable has no parent directory")
            .join("assets")
    }
}
