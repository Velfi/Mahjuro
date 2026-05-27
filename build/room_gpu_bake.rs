//! Batches room GI + shadow GPU bakes into a single `mahjuro-bake` invocation when possible.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use super::input_hash::{log_bake_timing, write_stamp_line};
use super::{room_gi_bake, room_shadow_bake};

pub fn emit_rerun_if_changed() {
    room_gi_bake::emit_rerun_if_changed();
    room_shadow_bake::emit_rerun_if_changed();
}

pub fn maybe_bake_room_gpu(repo: &Path, profile_dir: &Path) {
    let gi_skip = room_gi_bake::skip_bake_env();
    let shadow_skip = room_shadow_bake::skip_bake_env();

    if gi_skip {
        println!("cargo:warning=MAHJURO_SKIP_ROOM_GI_BAKE: skipping room GI probe bake");
    }
    if shadow_skip {
        println!("cargo:warning=MAHJURO_SKIP_ROOM_SHADOW_BAKE: skipping room shadow bake");
    }

    let gi = room_gi_bake::bake_status(repo);
    let shadow = room_shadow_bake::bake_status(repo);

    let need_gi = !gi_skip && !gi.up_to_date;
    let need_shadow = !shadow_skip && !shadow.up_to_date;

    if !gi_skip && gi.up_to_date {
        println!("cargo:info=room GI bake: inputs unchanged, skipping GPU bake");
    }
    if !shadow_skip && shadow.up_to_date {
        println!("cargo:info=room shadow bake: inputs unchanged, skipping GPU bake");
    }

    if !need_gi && !need_shadow {
        return;
    }

    let start = Instant::now();
    let exe = super::bake_tool::require_bake_exe(profile_dir);

    let mut kinds = Vec::new();
    if need_gi {
        kinds.push("gi");
    }
    if need_shadow {
        kinds.push("shadow");
    }
    let kinds_arg = kinds.join(",");

    println!(
        "cargo:warning=room GPU bake: running {kinds_arg} via {}",
        exe.display()
    );

    let mut cmd = Command::new(&exe);
    cmd.current_dir(repo);
    cmd.arg("--kinds").arg(&kinds_arg);

    if need_gi {
        room_gi_bake::ensure_out_dir(repo);
        cmd.args([
            "--gi-dir",
            room_gi_bake::out_dir(repo).to_str().unwrap_or(room_gi_bake::OUT_DIR),
            "--width",
            &room_gi_bake::BAKE_WIDTH.to_string(),
            "--height",
            &room_gi_bake::BAKE_HEIGHT.to_string(),
        ]);
    }
    if need_shadow {
        room_shadow_bake::ensure_out_dir(repo);
        cmd.args([
            "--shadow-dir",
            room_shadow_bake::out_dir(repo)
                .to_str()
                .unwrap_or(room_shadow_bake::OUT_DIR),
        ]);
    }

    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => panic!(
            "room GPU bake ({kinds_arg}) failed (exit {s}); fix headless init or set \
             MAHJURO_SKIP_ROOM_GI_BAKE=1 / MAHJURO_SKIP_ROOM_SHADOW_BAKE=1"
        ),
        Err(e) => panic!("failed to spawn room GPU bake ({kinds_arg}): {e}"),
    }

    if need_gi {
        write_stamp_line(&room_gi_bake::stamp_file(repo), &gi.hash).unwrap_or_else(|e| {
            panic!(
                "room GI bake: could not write stamp {}: {e}",
                room_gi_bake::stamp_file(repo).display()
            );
        });
    }
    if need_shadow {
        write_stamp_line(&room_shadow_bake::stamp_file(repo), &shadow.hash).unwrap_or_else(|e| {
            panic!(
                "room shadow bake: could not write stamp {}: {e}",
                room_shadow_bake::stamp_file(repo).display()
            );
        });
        println!("cargo:info=room shadow bake: wrote {}/*.msh", room_shadow_bake::OUT_DIR);
    }

    log_bake_timing(&format!("room GPU ({kinds_arg})"), start);
}
