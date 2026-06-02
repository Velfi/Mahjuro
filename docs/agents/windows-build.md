# Windows native build

## Prerequisites

- [Rust](https://rustup.rs/) (repo tracks stable; check `rust-toolchain.toml` if present).
- **Visual Studio 2019 or 2022** with **Desktop development with C++** (MSVC linker + Windows SDK).
- Steamworks SDK: set `STEAM_SDK_LOCATION` to the `sdk` directory inside your SDK tree (e.g. `C:\Users\zelda\steamworks_sdk_164\sdk`).

## Build

From a **Developer Command Prompt** (so `link.exe` and SDK libs are on `PATH`):

```powershell
$env:STEAM_SDK_LOCATION = "C:\Users\zelda\steamworks_sdk_164\sdk"
cargo build --release
```

`build.rs` copies `redistributable_bin\win64\steam_api64.dll` beside `target\release\mahjuro.exe`. Steam is loaded at runtime via `LoadLibraryW` (no `/DELAYLOAD`).

## Linker notes

- Do **not** force `rust-lld` for the main binary on MSVC (see `.cargo/config.toml`). Some VS Build Tools installs lack `atls.lib`, which `/DELAYLOAD` can pull in.
- Mahjuro does **not** enable wgpu’s `static-dxc` feature: prebuilt DXC static libs from `mach-dxcompiler-rs` require **VS 2022 (MSVC 1941+)** STL symbols. DX12 instead loads `dxcompiler.dll` when available.

## Graphics backends on Windows

Default when `WGPU_BACKEND` is unset: **DX12** (`crates/mahjuro-render`).

| Goal | Action |
| --- | --- |
| AMD driver issues / testing Vulkan | `$env:WGPU_BACKEND = "vulkan"` before launch |
| DX12 without bundled DXC | Install [DirectX Shader Compiler](https://github.com/microsoft/DirectXShaderCompiler) and ensure `dxcompiler.dll` is on `PATH`, or place it next to `mahjuro.exe` |
| Full SM 6.x + static DXC in the binary | Use VS 2022+, re-enable `static-dxc` on the workspace `wgpu` dependency, and accept the newer toolchain requirement |

Steam Deck / Proton: DX12 may fall back to FXC when `dxcompiler.dll` is missing; see root `Cargo.toml` comment on `wgpu` / Proton.
