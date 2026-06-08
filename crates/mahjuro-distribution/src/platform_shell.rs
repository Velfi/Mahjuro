//! Store-safe shell integration (reveal folders, export save panels).

use std::path::{Path, PathBuf};

/// Reveal `path` in the system file manager.
pub fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }

    #[cfg(all(feature = "macos-store", target_os = "macos"))]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSArray, NSURL};
        let path_str = path.to_string_lossy();
        let url =
            NSURL::fileURLWithPath_isDirectory(&objc2_foundation::NSString::from_str(&path_str), true);
        let urls = NSArray::from_retained_slice(std::slice::from_ref(&url));
        let workspace = NSWorkspace::sharedWorkspace();
        workspace.activateFileViewerSelectingURLs(&urls);
        return Ok(());
    }

    #[cfg(all(feature = "windows-store", target_os = "windows"))]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::{ILCreateFromPathW, SHOpenFolderAndSelectItems};

        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let item = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
            if item.is_null() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("ILCreateFromPathW failed for {}", path.display()),
                ));
            }
            SHOpenFolderAndSelectItems(item, None, 0)
                .map_err(|e| std::io::Error::other(format!("SHOpenFolderAndSelectItems: {e}")))?;
        }
        return Ok(());
    }

    #[cfg(all(target_os = "macos", not(feature = "macos-store")))]
    {
        let status = std::process::Command::new("open").arg(path).status()?;
        if status.success() {
            return Ok(());
        }
        return Err(std::io::Error::other(format!(
            "open exited with {status}"
        )));
    }
    #[cfg(all(target_os = "windows", not(feature = "windows-store")))]
    {
        let status = std::process::Command::new("explorer").arg(path).status()?;
        if status.success() {
            return Ok(());
        }
        return Err(std::io::Error::other(format!(
            "explorer exited with {status}"
        )));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let status = std::process::Command::new("xdg-open").arg(path).status()?;
        if status.success() {
            return Ok(());
        }
        return Err(std::io::Error::other(format!(
            "xdg-open exited with {status}"
        )));
    }
}

/// Ask the user where to save `default_name`; returns `None` if cancelled.
pub fn export_via_save_panel(default_name: &str) -> Option<PathBuf> {
    #[cfg(all(feature = "macos-store", target_os = "macos"))]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
        use objc2_foundation::NSString;

        let mtm = MainThreadMarker::new()?;
        let panel = NSSavePanel::savePanel(mtm);
        panel.setTitle(Some(&NSString::from_str("Export play stats")));
        panel.setNameFieldStringValue(&NSString::from_str(default_name));
        panel.setCanCreateDirectories(true);
        if panel.runModal() == NSModalResponseOK {
            if let Some(url) = panel.URL() {
                if let Some(path) = url.path() {
                    return Some(PathBuf::from(path.to_string()));
                }
            }
        }
        return None;
    }

    #[cfg(all(feature = "windows-store", target_os = "windows"))]
    {
        use windows::core::{w, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
        use windows::Win32::UI::Shell::{
            FileSaveDialog, FOS_OVERWRITEPROMPT, IFileSaveDialog, SIGDN_FILESYSPATH,
        };

        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let dialog: IFileSaveDialog = unsafe {
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?
        };
        let filter = COMDLG_FILTERSPEC {
            pszName: w!("HTML"),
            pszSpec: w!("*.html"),
        };
        unsafe {
            dialog.SetFileTypes(&[filter]).ok()?;
            dialog.SetFileName(&HSTRING::from(default_name)).ok()?;
            dialog.SetTitle(&HSTRING::from("Export play stats")).ok()?;
            dialog.SetOptions(FOS_OVERWRITEPROMPT).ok()?;
            if dialog.Show(None).is_ok() {
                let item = dialog.GetResult().ok()?;
                let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let path_str = unsafe { path.to_string().ok()? };
                return Some(PathBuf::from(path_str));
            }
        }
        return None;
    }

    #[cfg(not(any(
        all(feature = "macos-store", target_os = "macos"),
        all(feature = "windows-store", target_os = "windows")
    )))]
    {
        let _ = default_name;
        None
    }
}

/// Resolve export path: save panel on store SKUs, fixed Downloads path on Steam.
pub fn resolve_play_stats_export_path(profile_index: usize) -> Option<PathBuf> {
    if cfg!(any(feature = "macos-store", feature = "windows-store")) {
        let name = crate::platform_paths::play_stats_export_basename(profile_index);
        export_via_save_panel(&name)
    } else {
        Some(crate::platform_paths::play_stats_export_path(profile_index))
    }
}
