//! CPU-only bake: `data/relic_baked/<slug>.rlc` per relic (mask-cut albedo + relief + mesh).
//!
//! On success, refreshes `assets/data/relic_baked/.inputs_stamp` with the same FNV-1a
//! hash that `mahjuro`'s `build.rs` recomputes, so the next `cargo build` won't
//! panic with "relic RLC2 bake is out of date".
//!
//! Unchanged relics are skipped when their `<slug>.rlc.stamp` matches current inputs.

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::relic::{
    Relic, bootstrap_missing_sidecars, compute_entry_hash, force_relic_bake, read_relic_sidecar,
    relic_sidecar_path, write_relic_sidecar,
};

#[derive(Debug, Parser)]
#[command(
    name = "mahjuro-bake-relics",
    about = "Bake offline relic RLC2 payloads (mask-cut albedo + relief + mesh)"
)]
struct Cli {
    /// Rebake every relic even when sidecars match (also MAHJURO_FORCE_RELIC_BAKE=1).
    #[arg(long)]
    force: bool,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();

    let cli = Cli::parse();
    let force = cli.force || force_relic_bake();

    let repo = repo_root()?;
    let assets = std::env::var_os("MAHJURO_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo.join("assets"));

    // SAFETY: single-threaded bake binary; no concurrent env access.
    unsafe {
        std::env::set_var("MAHJURO_ASSETS", &assets);
    }
    mahjuro_assets::asset_path::init();

    let out_dir = assets.join("data/relic_baked");
    std::fs::create_dir_all(&out_dir)?;

    let defs = mahjuro_core::core::relic::all_relic_defs();
    let slugs: Vec<&str> = defs
        .iter()
        .map(|d| mahjuro_render::relic_bake::relic_slug(d.id))
        .collect();

    let global_status = Relic::bake_status(&repo);
    let global_stamp_ok = global_status.stamp_ok && global_status.outputs_ok;

    if !force && global_stamp_ok {
        let bootstrapped = bootstrap_missing_sidecars(&repo, &out_dir, &slugs)?;
        RelicBakeSummary {
            total: defs.len(),
            bootstrapped,
            stamp: Some(format!("{} already current", Relic::STAMP_PATH)),
            ..Default::default()
        }
        .print_tree();
        return Ok(());
    }

    let mut summary = RelicBakeSummary {
        total: defs.len(),
        ..Default::default()
    };
    let progress = relic_bake_progress(defs.len());

    for d in defs {
        let slug = mahjuro_render::relic_bake::relic_slug(d.id);
        progress.set_message(slug.to_string());
        let entry_hash = compute_entry_hash(&repo, slug);
        let sidecar_path = relic_sidecar_path(&out_dir, slug);
        let out = assets.join(mahjuro_render::relic_bake::baked_relic_asset_path(d.id));
        let sidecar = read_relic_sidecar(&sidecar_path);
        let rlc_ok =
            out.is_file() && mahjuro_render::relic_bake::validate_baked_relic(d.id).is_ok();

        if !force && sidecar.as_deref() == Some(entry_hash.as_str()) && rlc_ok {
            summary.unchanged += 1;
            progress.inc(1);
            continue;
        }

        if !force && sidecar.is_none() && rlc_ok && global_stamp_ok {
            write_relic_sidecar(&sidecar_path, &entry_hash)?;
            summary.bootstrapped += 1;
            progress.inc(1);
            continue;
        }

        let Some((msg, _mesh_build)) =
            mahjuro_render::relic_pipeline::decode_relic_assets(d.id, d.name)
        else {
            log::warn!(
                "skip {:?}: no source PNG at {} or {}",
                d.id,
                d.id.render_texture_path(),
                d.id.source_object_path()
            );
            summary.skipped += 1;
            progress.inc(1);
            continue;
        };
        let bytes = mahjuro_render::relic_bake::encode_baked_relic(&msg)?;
        std::fs::write(&out, &bytes)?;
        write_relic_sidecar(&sidecar_path, &entry_hash)?;
        summary.baked += 1;
        summary.bytes_written += bytes.len();
        progress.inc(1);
    }
    progress.finish_and_clear();

    if summary.skipped == 0 {
        let stamped = Relic::write_stamp(&repo)?;
        summary.stamp = Some(format!(
            "{} ({})",
            stamped.stamp_path.display(),
            stamped.hash
        ));
    } else {
        log::warn!(
            "{} skipped relic(s); leaving {} alone so build.rs still flags the gap",
            summary.skipped,
            Relic::STAMP_PATH
        );
    }
    summary.print_tree();
    Ok(())
}

#[derive(Default)]
struct RelicBakeSummary {
    total: usize,
    baked: usize,
    unchanged: usize,
    bootstrapped: usize,
    skipped: usize,
    bytes_written: usize,
    stamp: Option<String>,
}

impl RelicBakeSummary {
    fn print_tree(&self) {
        println!("relic bake summary");
        println!(
            "├─ total: {} baked, {} unchanged, {} bootstrapped, {} skipped, {} planned",
            self.baked, self.unchanged, self.bootstrapped, self.skipped, self.total
        );
        println!("├─ written: {}", format_bytes(self.bytes_written));
        match &self.stamp {
            Some(stamp) => println!("└─ stamp: {stamp}"),
            None => println!("└─ stamp: unchanged due to skipped relics"),
        }
    }
}

fn relic_bake_progress(total: usize) -> ProgressBar {
    let progress = ProgressBar::new(total as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {msg:.48} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len}",
        )
        .expect("valid progress template")
        .progress_chars("=>-"),
    );
    progress
}

fn format_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

/// Repo root with no `..` components. The build script uses `CARGO_MANIFEST_DIR`
/// of `mahjuro` (already canonical); we mirror that by walking the parent chain
/// rather than `join("../..")`, since `Fnv64::write_path_key` hashes the literal
/// path string and any `..` would silently desync from `build.rs`'s digest.
fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
