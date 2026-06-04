// On Windows, compile as a GUI ("windows") subsystem app in release builds so
// the OS doesn't allocate a console window (the "debug terminal" that pops up
// when launched from Steam). Debug builds keep the console subsystem so
// stdout/stderr logging stays visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    mahjuro::run()
}
