# macOS shipping — dynamic libraries

The shipped **`Mahjuro.app`** must place vendored `.dylib` files in **`Contents/MacOS/`** next to `mahjuro`. System frameworks (`Metal`, `AppKit`, …) are not bundled.

## What to bundle

| Library | Why |
|--------|-----|
| **`libsteam_api.dylib`** | Linked as `@loader_path/libsteam_api.dylib`. Copied next to the binary by [`build.rs`](../../build.rs) during `cargo build`; **packaging must also copy it** into the app (CI historically missed this once). |
| **SDL3** | Linked **statically** via `sdl3` feature `build-from-source-static` — no `libSDL3` dylib in the bundle. |

Keep **[`.github/workflows/release.yml`](../../.github/workflows/release.yml)** and **[`scripts/package-macos.sh`](../../scripts/package-macos.sh)** aligned whenever this layout changes.

## Verification

After changing native deps, check the release binary:

```bash
otool -L target/release/mahjuro
```

Bundle anything under **`@rpath`**, **`@loader_path`**, or non-system paths. Re-check after adding crates that ship shared libraries.
