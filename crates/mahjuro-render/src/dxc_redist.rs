//! Runtime lookup for `dxcompiler.dll` (mirrors `build/dxc_redist.rs`).

use std::path::PathBuf;

#[cfg(target_os = "windows")]
pub(crate) fn resolve_dxcompiler_dll() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("MAHJURO_DXC_REDIST").filter(|s| !s.is_empty()) {
        let dll = PathBuf::from(raw).join("dxcompiler.dll");
        if dll.is_file() {
            return Some(dll);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let dll = dir.join("dxcompiler.dll");
            if dll.is_file() {
                return Some(dll);
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let dll = cwd.join("third_party/dxc-redist/x64/dxcompiler.dll");
        if dll.is_file() {
            return Some(dll);
        }
    }

    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Redist\D3D\x64\dxcompiler.dll");
    if kits.is_file() {
        return Some(kits);
    }

    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn resolve_dxcompiler_dll() -> Option<PathBuf> {
    None
}
