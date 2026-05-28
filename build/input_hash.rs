//! Shared FNV-1a input fingerprinting and bake timing for `build.rs` hooks.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// FNV-1a 64-bit (stable across toolchains for build stamps).
pub struct Fnv64 {
    state: u64,
}

impl Fnv64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    pub fn new() -> Self {
        Self {
            state: Self::OFFSET,
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    pub fn write_path_key(&mut self, path: &Path) {
        self.write(path.to_string_lossy().as_bytes());
        self.write(b"\0");
    }

    pub fn finish(self) -> u64 {
        self.state
    }

    pub fn finish_hex(self) -> String {
        format!("{:016x}", self.finish())
    }
}

/// Mix every listed path into `h`. Missing paths contribute only the path key.
pub fn hash_paths(h: &mut Fnv64, paths: &[PathBuf]) {
    for path in paths {
        h.write_path_key(path);
        if path.is_file()
            && let Ok(bytes) = fs::read(path)
        {
            h.write(&bytes);
        }
    }
}

/// Depth-first walk of `root`, skipping paths where `skip(rel_from_root)` is true.
/// `rel_from_root` uses `/` separators and is empty at `root`.
pub fn hash_tree(
    h: &mut Fnv64,
    root: &Path,
    rel_prefix: &str,
    skip: &impl Fn(&str) -> bool,
) {
    if skip(rel_prefix) {
        return;
    }
    if root.is_file() {
        h.write_path_key(root);
        if let Ok(bytes) = fs::read(root) {
            h.write(&bytes);
        }
        return;
    }
    if !root.is_dir() {
        return;
    }
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    let mut children: Vec<_> = read.filter_map(|e| e.ok()).collect();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let rel = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        hash_tree(h, &entry.path(), &rel, skip);
    }
}

pub fn read_stamp_line(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let line = s.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

pub fn write_stamp_line(path: &Path, hash: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{hash}\n"))
}

pub fn outputs_present(out_dir: &Path, rooms: &[&str], ext: &str) -> bool {
    rooms
        .iter()
        .all(|room| out_dir.join(format!("{room}.{ext}")).is_file())
}

/// Fail the build when committed bake outputs or their stamp do not match current inputs.
pub fn assert_committed_bake_current(c: CommittedBakeCheck<'_>) {
    if c.stamp_ok && c.outputs_ok {
        println!("cargo:info={}: committed bake matches inputs", c.label);
        return;
    }

    let detail = if !c.stamp_ok {
        format!(
            "  {} is missing or stale (expected hash {})",
            c.stamp_path, c.expected_hash
        )
    } else {
        format!("  baked outputs missing or incomplete under {}/", c.outputs_dir)
    };

    let message = format!(
        concat!(
            "{label} is out of date.\n\n",
            "{detail}\n\n",
            "To fix (needs a GPU):\n\n",
            "1. Build the offline baker:\n",
            "   {skip_env}=1 {build_tool_cmd}\n\n",
            "2. Rebake and refresh the stamp:\n",
            "   {rebake_cmd}\n\n",
            "3. Commit the baked files:\n",
            "   git add {commit_paths}",
        ),
        label = c.label,
        detail = detail,
        skip_env = c.skip_env,
        build_tool_cmd = c.build_tool_cmd,
        rebake_cmd = c.rebake_cmd,
        commit_paths = c.commit_paths,
    );
    panic!("{message}");
}

pub struct CommittedBakeCheck<'a> {
    pub label: &'a str,
    pub stamp_path: &'a str,
    pub outputs_dir: &'a str,
    pub commit_paths: &'a str,
    pub expected_hash: &'a str,
    pub stamp_ok: bool,
    pub outputs_ok: bool,
    pub skip_env: &'a str,
    pub build_tool_cmd: &'a str,
    pub rebake_cmd: &'a str,
}

pub fn log_bake_timing(label: &str, start: Instant) {
    let secs = start.elapsed().as_secs_f64();
    println!("cargo:info=bake timing: {label} {secs:.2}s");
}
