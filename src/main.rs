// Release builds on Windows: detach from the console so launching the .exe
// doesn't pop a black terminal behind the game window. Debug builds keep the
// console so `log` output is visible during development.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    mahjuro::run()
}
