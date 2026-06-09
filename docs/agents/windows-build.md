# Windows native build

## Prerequisites

- [Rust](https://rustup.rs/) (repo tracks stable; check `rust-toolchain.toml` if present).
- **Visual Studio 2019 or 2022** with **Desktop development with C++** (MSVC linker + Windows SDK).
- Steamworks SDK: set `STEAM_SDK_LOCATION` to the `sdk` directory inside your SDK tree (e.g. `C:\Users\zelda\steamworks_sdk_164\sdk`).

## Build

From a **Developer Command Prompt** (so `link.exe` and SDK libs are on `PATH`):

```powershell
$env:STEAM_SDK_LOCATION = "C:\Users\zelda\steamworks_sdk_164\sdk"
.\scripts\fetch-dxc-redist.ps1
cargo build --release
```

`build.rs` copies runtime dependencies beside `target\<profile>\mahjuro.exe`:

| File | Purpose |
| --- | --- |
| `steam_api64.dll` | Steamworks (when `dist-steam` is enabled); from `$STEAM_SDK_LOCATION` or `steamworks-sys` |
| `dxcompiler.dll`, `dxil.dll` | DirectX Shader Compiler (DXC) for DX12 — avoids FXC fallback |

**Packaging** (release zip, NSIS installer, Steam depot, MS Store MSIX) must ship all of the above next to `mahjuro.exe`. Release CI runs `scripts/fetch-dxc-redist.ps1` before the release build.

### DXC redist (shader compiler)

Mahjuro does **not** enable wgpu’s `static-dxc` feature (see [Linker notes](#linker-notes)). On DX12, wgpu prefers **DXC** (`dxcompiler.dll` + `dxil.dll`). If those are missing, it falls back to **FXC** via `d3dcompiler_47.dll` — slower, SM 5.1 limits, and a common pain point on Proton/Steam Deck.

**Recommended — fetch script** (pinned NuGet package; version in `third_party/dxc-redist/VERSION`):

```powershell
.\scripts\fetch-dxc-redist.ps1
```

This downloads [Microsoft.Direct3D.DXC](https://www.nuget.org/packages/Microsoft.Direct3D.DXC) and installs x64 binaries under `third_party/dxc-redist/x64/`. `build.rs` copies them into the cargo profile directory on the next build.

**Alternatives:**

| Source | How |
| --- | --- |
| Windows SDK | `C:\Program Files (x86)\Windows Kits\10\Redist\D3D\x64\` (auto-detected by `build.rs` if the pair exists) |
| GitHub | [DirectXShaderCompiler releases](https://github.com/microsoft/DirectXShaderCompiler/releases) — use the x64 `dxcompiler.dll` + `dxil.dll` from the zip |
| Custom path | `$env:MAHJURO_DXC_REDIST = "C:\path\to\x64"` before `cargo build` |

Minimum DXC version for wgpu: **v1.8.2502**. Do not download DLLs from third-party mirror sites — use NuGet, GitHub, or the Windows SDK only.

Verify DXC at runtime (debug logging): look for `Using dynamic DXC for shader compilation` instead of `Using FXC for shader compilation`.

## Linker notes

- Do **not** force `rust-lld` for the main binary on MSVC (see `.cargo/config.toml`). Some VS Build Tools installs lack `atls.lib`, which `/DELAYLOAD` can pull in.
- Mahjuro does **not** enable wgpu’s `static-dxc` feature: prebuilt DXC static libs from `mach-dxcompiler-rs` require **VS 2022 (MSVC 1941+)** STL symbols. DX12 instead loads `dxcompiler.dll` when available.

## Graphics backends on Windows

Default when `WGPU_BACKEND` is unset: **DX12** (`crates/mahjuro-render`).

| Goal | Action |
| --- | --- |
| AMD driver issues / testing Vulkan | `$env:WGPU_BACKEND = "vulkan"` before launch |
| Force FXC (CI smoke test) | `$env:WGPU_DX12_COMPILER = "fxc"` |
| Full SM 6.x + static DXC in the binary | Use VS 2022+, re-enable `static-dxc` on the workspace `wgpu` dependency, and accept the newer toolchain requirement |

Steam Deck / Proton: prefer Vulkan (`WGPU_BACKEND=vulkan`) or ship DXC next to the exe; DX12 without `dxcompiler.dll` often hits FXC and shader limits.
