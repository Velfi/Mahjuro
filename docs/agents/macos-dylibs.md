# macOS shipping — dynamic libraries

The shipped **`Mahjuro.app`** must place vendored `.dylib` files in **`Contents/MacOS/`** next to `mahjuro`. System frameworks (`Metal`, `AppKit`, …) are not bundled.

## What to bundle

| Library | Why |
|--------|-----|
| **`libsteam_api.dylib`** | Linked as `@loader_path/libsteam_api.dylib`. Copied next to the binary by [`build.rs`](../../build.rs) during `cargo build`; **packaging must also copy it** into the app (CI historically missed this once). |
| **`libSDL3.0.dylib`** | Linked as `@rpath/libSDL3.0.dylib` from `sdl3` (dynamic, build-from-source). **`build.rs` adds `-Wl,-rpath,@loader_path`** so resolution uses the executable directory. **Packaging must copy** the dylib; universal builds **`lipo`** the arm64 and x86_64 copies, then run **`install_name_tool -id @loader_path/libSDL3.0.dylib`** on the bundled file. |

Keep **[`.github/workflows/release.yml`](../../.github/workflows/release.yml)** and **[`scripts/package-macos.sh`](../../scripts/package-macos.sh)** aligned whenever this layout changes.

## Sparkle

**`Sparkle.framework`** lives under **`Contents/Frameworks/`**, loaded at runtime (see [`macos_updater.rs`](../../src/macos_updater.rs)). It is not a `Contents/MacOS/` dylib dependency of the main binary.

## Verification

After changing native deps, check the release binary:

```bash
otool -L target/release/mahjuro
```

Bundle anything under **`@rpath`**, **`@loader_path`**, or non-system paths. Re-check after adding crates that ship shared libraries.
