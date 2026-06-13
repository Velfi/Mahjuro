//! Single SDL3 context: window, event pump, gamepads (wgpu stays separate).

use rustc_hash::{FxHashMap, FxHashSet};

use sdl3::gamepad::Gamepad;
use sdl3::joystick::JoystickId;
use sdl3::video::{FullscreenType, Window, WindowBuilder, WindowFlags};
use sdl3::{EventPump, GamepadSubsystem, JoystickSubsystem, Sdl, VideoSubsystem};

pub struct SdlShell {
    pub(crate) _sdl: Sdl,
    pub(crate) _video: VideoSubsystem,
    pub window: Window,
    pub(crate) pump: EventPump,
    pub(crate) gamepad: GamepadSubsystem,
    /// Rumble is applied via the gamepad API but SDL documents that
    /// [`SDL_UpdateJoysticks`](https://wiki.libsdl.org/SDL3/SDL_RumbleGamepad) must run for effects to reach hardware.
    pub(crate) joystick: JoystickSubsystem,
    pub(crate) pads: FxHashMap<JoystickId, Gamepad>,
    pub(crate) lt_prev: FxHashMap<JoystickId, f32>,
    pub(crate) rt_prev: FxHashMap<JoystickId, f32>,
    /// Joystick IDs we already logged as non-gamepad (avoid spam each frame).
    non_gamepad_logged: FxHashSet<JoystickId>,
    /// Pads that report no rumble or where `set_rumble` failed — skip further attempts.
    rumble_unavailable: FxHashSet<JoystickId>,
    /// Subset of `rumble_unavailable` we already logged a runtime failure for.
    rumble_fail_logged: FxHashSet<JoystickId>,
}

impl SdlShell {
    /// Creates a resizable windowed surface (or true fullscreen when `SteamTenfoot` is set).
    /// Call [`Self::apply_borderless_from_settings`] after startup prefs are loaded and the
    /// window is shown — borderless is not applied here so macOS hide/show during GPU init
    /// does not drop desktop fullscreen before the player sees the window.
    pub fn new(title: &str, width: u32, height: u32) -> anyhow::Result<Self> {
        let _sdl = sdl3::init().map_err(anyhow::Error::from)?;
        #[cfg(target_os = "macos")]
        sdl3::hint::set("SDL_VIDEO_MACOSX_METAL_LAYER", "1");

        // SDL HIDAPI for Valve controllers (Steam Deck, Steam Controller) so
        // devices present as standard gamepads with stable mappings.
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI", "1");
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_STEAM", "1");
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_STEAMDECK", "1");

        let _video = _sdl.video().map_err(anyhow::Error::from)?;
        let gamepad = _sdl.gamepad().map_err(anyhow::Error::from)?;
        let joystick = _sdl.joystick().map_err(anyhow::Error::from)?;

        let tenfoot = std::env::var_os("SteamTenfoot").is_some();
        let (win_w, win_h) = if tenfoot {
            (width, height)
        } else {
            Self::clamp_launch_window_size(&_video, width, height)
        };
        let mut wb = WindowBuilder::new(&_video, title, win_w, win_h);
        wb.resizable().high_pixel_density();
        if tenfoot {
            wb.fullscreen();
        } else {
            wb.position_centered();
        }

        #[cfg(target_os = "macos")]
        {
            use sdl3::video::WindowFlags;
            wb.set_flags(
                WindowFlags::METAL | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
            );
            wb.metal_view();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            wb.vulkan();
        }
        #[cfg(target_os = "windows")]
        {
            use sdl3::video::WindowFlags;
            wb.set_flags(WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY);
            // Do NOT set `SDL_WINDOW_VULKAN` here: wgpu creates its Vulkan surface from the raw
            // HWND (`Instance::create_surface_unsafe` → `vkCreateWin32SurfaceKHR`), so the flag is
            // not required for the Vulkan WSI path. Setting it changes the Win32 window class SDL
            // registers, which drops mouse events through the default `WindowProc`.
        }

        let window = wb
            .build()
            .map_err(|e| anyhow::anyhow!("SDL_CreateWindow: {e}"))?;

        let pump = _sdl.event_pump().map_err(anyhow::Error::from)?;

        let mut shell = Self {
            _sdl,
            _video,
            window,
            pump,
            gamepad,
            joystick,
            pads: FxHashMap::default(),
            lt_prev: FxHashMap::default(),
            rt_prev: FxHashMap::default(),
            non_gamepad_logged: FxHashSet::default(),
            rumble_unavailable: FxHashSet::default(),
            rumble_fail_logged: FxHashSet::default(),
        };
        shell.refresh_gamepads();

        Ok(shell)
    }

    /// Enter borderless desktop fullscreen when `borderless_fullscreen` is set in prefs.
    /// Returns true when the window mode changed (caller should resize the swapchain).
    pub fn apply_borderless_from_settings(
        &mut self,
        borderless_fullscreen: bool,
    ) -> anyhow::Result<bool> {
        let tenfoot = std::env::var_os("SteamTenfoot").is_some();
        if tenfoot || !borderless_fullscreen || self.desktop_fullscreen_on() {
            return Ok(false);
        }
        self.set_desktop_fullscreen(true)?;
        Ok(true)
    }

    fn clamp_launch_window_size(video: &VideoSubsystem, w: u32, h: u32) -> (u32, u32) {
        let Ok(display) = video.get_primary_display() else {
            return (w, h);
        };
        let Ok(bounds) = display.get_usable_bounds() else {
            return (w, h);
        };
        let max_w = bounds.width().max(1);
        let max_h = bounds.height().max(1);
        (w.min(max_w), h.min(max_h))
    }

    pub fn show_cursor(&self, show: bool) {
        self._sdl.mouse().show_cursor(show);
    }

    /// Raise this window and request keyboard focus (SDL `SDL_RaiseWindow`).
    ///
    /// Terminal launches on macOS often leave the game visible but behind the
    /// shell until the user clicks in; call after the window is shown.
    pub fn raise_to_foreground(&mut self) {
        let _ = self.window.raise();
    }

    pub fn prepare_gamepad_frame(&mut self) {
        self.gamepad.update();
        self.joystick.update();
        self.refresh_gamepads();
    }

    /// Call after any `Gamepad::set_rumble` so SDL pushes the effect to the driver this frame.
    #[inline]
    pub fn sync_gamepad_rumble_output(&self) {
        self.gamepad.update();
        self.joystick.update();
    }

    /// Apply dual-motor rumble to connected pads that support it.
    /// Returns true when at least one pad accepted the effect.
    pub(crate) fn set_gamepad_rumble(&mut self, low: u16, high: u16, duration_ms: u32) -> bool {
        if low == 0 && high == 0 {
            return false;
        }
        let mut any = false;
        for (&id, gp) in self.pads.iter_mut() {
            if self.rumble_unavailable.contains(&id) {
                continue;
            }
            if !unsafe { gp.has_rumble() } {
                self.rumble_unavailable.insert(id);
                continue;
            }
            match gp.set_rumble(low, high, duration_ms) {
                Ok(()) => any = true,
                Err(e) => {
                    self.rumble_unavailable.insert(id);
                    if self.rumble_fail_logged.insert(id) {
                        log::debug!("gamepad rumble unavailable (id={}): {e}", id.0);
                    }
                }
            }
        }
        if any {
            self.sync_gamepad_rumble_output();
        }
        any
    }

    pub(crate) fn stop_gamepad_rumble(&mut self) {
        let mut any = false;
        for (&id, gp) in self.pads.iter_mut() {
            if self.rumble_unavailable.contains(&id) {
                continue;
            }
            if gp.set_rumble(0, 0, 1).is_ok() {
                any = true;
            }
        }
        if any {
            self.sync_gamepad_rumble_output();
        }
    }

    pub fn refresh_gamepads(&mut self) {
        let Ok(ids) = self.gamepad.gamepads() else {
            return;
        };
        self.pads.retain(|id, _| ids.contains(id));
        self.lt_prev.retain(|id, _| ids.contains(id));
        self.rt_prev.retain(|id, _| ids.contains(id));
        self.non_gamepad_logged.retain(|id| ids.contains(id));
        self.rumble_unavailable.retain(|id| ids.contains(id));
        self.rumble_fail_logged.retain(|id| ids.contains(id));
        for &id in &ids {
            if self.pads.contains_key(&id) {
                continue;
            }
            if !self.gamepad.is_gamepad(id) {
                if !self.non_gamepad_logged.contains(&id) {
                    self.non_gamepad_logged.insert(id);
                }
                continue;
            }
            match self.gamepad.open(id) {
                Ok(gp) => {
                    let name = gp.name().unwrap_or_else(|| "(unknown)".into());
                    let rumble = unsafe { gp.has_rumble() };
                    log::debug!(
                        "SDL gamepad opened: id={} name={name:?} SDL_PROP_GAMEPAD_CAP_RUMBLE={rumble}",
                        id.0
                    );
                    if !rumble {
                        self.rumble_unavailable.insert(id);
                        log::warn!(
                            "This gamepad reports no rumble to SDL — force feedback will not run (driver/SDL limits; see SDL joystick hints)."
                        );
                    }
                    self.lt_prev.insert(id, 0.0);
                    self.rt_prev.insert(id, 0.0);
                    self.pads.insert(id, gp);
                }
                Err(e) => {
                    log::warn!("SDL_OpenGamepad failed for id {}: {e}", id.0);
                }
            }
        }
    }

    pub fn drawable_size(&self) -> (u32, u32) {
        self.window.size_in_pixels()
    }

    /// True when this window should simulate and render.
    ///
    /// We only gate on **actual occlusion** (minimized / hidden). Using keyboard or mouse
    /// focus here is too strict on macOS: the window can be fully visible behind another app
    /// with neither `INPUT_FOCUS` nor `MOUSE_FOCUS` until the user clicks in — skipping
    /// `frame_tick` in that state never presented a swapchain frame, so the Metal layer stayed
    /// black through splash → main menu. Gamepad presence used to paper over the same gap.
    ///
    /// When false, the SDL main loop skips per-frame simulation and rendering so the game does
    /// not keep running while minimized or hidden.
    pub fn window_is_foreground(&self) -> bool {
        let flags = WindowFlags::from(self.window.window_flags());
        !flags.intersects(WindowFlags::MINIMIZED | WindowFlags::HIDDEN)
    }

    /// Map SDL window-coordinate mouse position to drawable pixels (HiDPI).
    ///
    /// Use `SDL_GetWindowPixelDensity` (= `pixel_size / window_size`), not
    /// `SDL_GetWindowDisplayScale`. The latter folds in the user's display content scale (the
    /// Windows "Scale & layout" 125/150 % setting), so at 150 % a click at the right edge of the
    /// window lands ~1.5× past the right edge of the backbuffer and misses every UI rect.
    pub fn event_xy_to_pixels(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.window.pixel_density();
        (x * s, y * s)
    }

    pub fn desktop_fullscreen_on(&self) -> bool {
        matches!(
            self.window.fullscreen_state(),
            FullscreenType::Desktop | FullscreenType::True
        )
    }

    pub fn set_desktop_fullscreen(&mut self, on: bool) -> anyhow::Result<()> {
        self.window
            .set_fullscreen(on)
            .map_err(anyhow::Error::from)?;
        Ok(())
    }
}
