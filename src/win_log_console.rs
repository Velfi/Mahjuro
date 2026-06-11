//! Windows release builds link as a GUI subsystem binary (`windows_subsystem =
//! "windows"`), so stderr is discarded unless we attach to the launching console
//! or redirect logs to a file.

#[cfg(all(windows, not(debug_assertions)))]
use std::io::LineWriter;

/// When `RUST_LOG` is set on a Windows release build, try to attach to the parent
/// console and return a writer for it. Otherwise return a default log path under
/// the Mahjuro data directory.
#[cfg(all(windows, not(debug_assertions)))]
pub fn prepare_rust_log_output() -> RustLogOutput {
    if std::env::var_os("RUST_LOG").is_none() {
        return RustLogOutput::None;
    }
    if try_attach_parent_console() {
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            return RustLogOutput::Console(LineWriter::new(file));
        }
    }
    RustLogOutput::File(mahjuro_distribution::PlatformPaths::data_root().join("mahjuro.log"))
}

#[cfg(all(windows, not(debug_assertions)))]
fn try_attach_parent_console() -> bool {
    const ERROR_ACCESS_DENIED: u32 = 5;

    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

        AttachConsole(ATTACH_PARENT_PROCESS) != 0
            || windows_sys::Win32::Foundation::GetLastError() == ERROR_ACCESS_DENIED
    }
}

#[cfg(not(all(windows, not(debug_assertions))))]
pub fn prepare_rust_log_output() -> RustLogOutput {
    RustLogOutput::None
}

pub enum RustLogOutput {
    None,
    #[cfg(all(windows, not(debug_assertions)))]
    Console(LineWriter<std::fs::File>),
    File(std::path::PathBuf),
}
