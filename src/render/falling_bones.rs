#![allow(dead_code)]
//! Physical scoring bones that tumble onto the play space during a cascade.
//!
//! Spawned as each scoring step reveals (chips or mult), each bone falls under
//! a constant gravity in world-y, tumbling on three axes, lands on the table
//! plane, and is cleared when the cascade ends. Pixel-space (px, py) is
//! pinned per bone — only world_y evolves under gravity — so the renderer's
//! [`crate::render::world_space::pixel_to_world`] places each bone at a stable spot on the table
//! while it falls. The renderer reuses the bone-tablet mesh + lit-mesh
//! pipeline; only the per-instance model matrix and tint vary.
//!
//! [`FallingBonePlacement::rotation`](crate::render::draw_cmd::FallingBonePlacement::rotation)
//! is consumed as **`Ry * Rx * Rz`** by the renderer — see
//! [`crate::render::table_transform::rot_ry_rx_rz_rad`].

use rand::RngExt;

use crate::render::draw_cmd::{CascadeTokenKind, FallingBonePlacement};

/// World-y gravity acceleration (units / s²). Negative because world_y grows
/// upward in [`crate::render::world_space::pixel_to_world`]'s output. Sized in concert with `SPAWN_HEIGHT`
/// so a fresh bone takes roughly 0.85s to hit the table — long enough to
/// read the tumble, short enough that a multi-step cascade isn't waiting on
/// the previous burst to finish.
const GRAVITY: f32 = -1400.0;

/// World-y of the table plane — bones rest here once they land.
const TABLE_Y: f32 = 0.0;

/// Initial height (world-y) at which fresh bones spawn. Held high above the
/// play space so the bones visibly drop in from above the camera frame
/// rather than just popping into existence near the table.
const SPAWN_HEIGHT: f32 = 520.0;

/// How long a landed bone lingers before fading out (seconds).
const REST_LIFETIME: f32 = 0.9;

/// How long a single burst's bones are spread out across (seconds). Each
/// bone in a burst gets a random delay in `[0, BURST_SPREAD]` so the spawn
/// reads as a random stream falling from the sky rather than a single
/// simultaneous pop. Tuned a hair shorter than the cascade step duration so
/// the stream finishes spawning before the next step's burst arrives.
const BURST_SPREAD: f32 = 0.45;

/// Snapshot of a landed bone's 2D footprint + top surface, used by the
/// stacking pass so an in-flight bone can come to rest on top of an
/// already-landed one whose horizontal AABB it overlaps.
#[derive(Clone, Copy, Debug)]
struct LandedFootprint {
    px: f32,
    py: f32,
    half_w: f32,
    half_d: f32,
    /// World-y of the top face of this landed bone.
    top_y: f32,
}

/// Walk the landed-bone snapshot and return the highest top-surface y
/// value of any bone whose 2D footprint overlaps `b`'s footprint. Falls
/// back to `floor_y` (the table plane) when nothing overlaps.
fn highest_support_below(landed: &[LandedFootprint], b: &FallingBone, floor_y: f32) -> f32 {
    let half_w = b.extents[0] * 0.5;
    let half_d = b.extents[2] * 0.5;
    let mut best = floor_y;
    for l in landed {
        // 2D AABB overlap in (px, py) plane.
        let dx = (b.px - l.px).abs();
        let dy = (b.py - l.py).abs();
        if dx < half_w + l.half_w && dy < half_d + l.half_d && l.top_y > best {
            best = l.top_y;
        }
    }
    best
}

/// One physical scoring bone in flight or at rest on the table.
#[derive(Clone, Debug)]
struct FallingBone {
    /// Pinned screen-pixel anchor (maps to world xz via [`crate::render::world_space::pixel_to_world`]).
    px: f32,
    py: f32,
    /// Height above the table — falls under `GRAVITY` until it hits `TABLE_Y`.
    world_y: f32,
    /// Vertical velocity in world units / second.
    vy: f32,
    /// Tumble angles around each axis (radians).
    rot_x: f32,
    rot_y: f32,
    rot_z: f32,
    /// Angular velocities (radians / second). Damped slightly each frame so
    /// landed bones come to rest instead of spinning forever.
    rvx: f32,
    rvy: f32,
    rvz: f32,
    /// Box dimensions (width × thickness × depth) in world units.
    extents: [f32; 3],
    /// Tint axis — drives the per-instance color.
    kind: CascadeTokenKind,
    /// Once the bone has landed, this counts down from `REST_LIFETIME` and
    /// drives the alpha fade. `None` while still in flight.
    rest_remaining: Option<f32>,
}

/// A bone scheduled to spawn after `delay` seconds. Carries the anchor +
/// kind so the burst's stream of spawns all aim at the same screen pocket
/// even though they enter the world spread out over time.
#[derive(Clone, Debug)]
struct PendingSpawn {
    delay: f32,
    anchor_px: f32,
    anchor_py: f32,
    kind: CascadeTokenKind,
}

pub struct FallingBoneSystem {
    bones: Vec<FallingBone>,
    pending: Vec<PendingSpawn>,
}

impl FallingBoneSystem {
    pub fn new() -> Self {
        Self {
            bones: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Queue a burst of `count` bones aimed at the same screen-pixel anchor.
    /// Each bone gets an independent random delay in `[0, BURST_SPREAD]` so
    /// the burst falls as a random stream from the sky instead of all
    /// arriving on the same frame. The anchor is the cascade token's
    /// screen position so each chips / mult reveal rains its own stream
    /// onto the play area below.
    pub fn burst(&mut self, anchor_px: f32, anchor_py: f32, count: usize, kind: CascadeTokenKind) {
        let mut rng = rand::rng();
        for _ in 0..count {
            self.pending.push(PendingSpawn {
                delay: rng.random::<f32>() * BURST_SPREAD,
                anchor_px,
                anchor_py,
                kind,
            });
        }
    }

    /// Materialize a single bone at the given anchor with randomized
    /// landing offset, scale, height, and tumble. Called by `update()` for
    /// every pending spawn whose timer has elapsed.
    fn spawn_one(&mut self, anchor_px: f32, anchor_py: f32, kind: CascadeTokenKind) {
        let mut rng = rand::rng();
        // Cluster the landing spots horizontally around the anchor and
        // drop them onto the play area (positive py = lower on screen).
        let dx = (rng.random::<f32>() - 0.5) * 220.0;
        let dy = rng.random::<f32>() * 140.0 + 30.0;
        // Slight extent variation so the pile doesn't look uniform.
        let scale = 0.9 + rng.random::<f32>() * 0.4;
        let w = 220.0 * scale;
        let t = 90.0 * scale;
        let d = 190.0 * scale;
        self.bones.push(FallingBone {
            px: anchor_px + dx,
            py: anchor_py + dy,
            world_y: SPAWN_HEIGHT + rng.random::<f32>() * 120.0,
            // Small random initial downward kick so they don't all start
            // from rest at exactly the same height.
            vy: -(rng.random::<f32>() * 20.0),
            rot_x: rng.random::<f32>() * std::f32::consts::TAU,
            rot_y: rng.random::<f32>() * std::f32::consts::TAU,
            rot_z: rng.random::<f32>() * std::f32::consts::TAU,
            rvx: (rng.random::<f32>() - 0.5) * 14.0,
            rvy: (rng.random::<f32>() - 0.5) * 10.0,
            rvz: (rng.random::<f32>() - 0.5) * 14.0,
            extents: [w, t, d],
            kind,
            rest_remaining: None,
        });
    }

    /// Advance the simulation by `dt` seconds. Bones in flight integrate
    /// gravity + tumble; landed bones tick down their rest timer and despawn
    /// when it reaches zero. Newly-landed bones rest *on top* of any
    /// previously-landed bones whose 2D footprint they overlap, so the pile
    /// stacks instead of inter-penetrating.
    pub fn update(&mut self, dt: f32) {
        // Drip pending spawns into the live pool as their timers expire.
        // Walk in reverse so swap_remove keeps the iteration cheap and the
        // remaining indices stable.
        let mut i = self.pending.len();
        while i > 0 {
            i -= 1;
            self.pending[i].delay -= dt;
            if self.pending[i].delay <= 0.0 {
                let p = self.pending.swap_remove(i);
                self.spawn_one(p.anchor_px, p.anchor_py, p.kind);
            }
        }
        // Landed footprints for stacking: must be collected before the
        // mutable pass over `self.bones` (collision + integration).
        let landed_snapshot: Vec<LandedFootprint> = self
            .bones
            .iter()
            .filter(|b| b.rest_remaining.is_some())
            .map(|b| LandedFootprint {
                px: b.px,
                py: b.py,
                half_w: b.extents[0] * 0.5,
                half_d: b.extents[2] * 0.5,
                top_y: b.world_y + b.extents[1] * 0.5,
            })
            .collect();
        for b in &mut self.bones {
            if let Some(rest) = b.rest_remaining.as_mut() {
                *rest -= dt;
                // Bleed angular velocity out as the bone settles.
                b.rvx *= 0.85;
                b.rvy *= 0.85;
                b.rvz *= 0.85;
                b.rot_x += b.rvx * dt;
                b.rot_y += b.rvy * dt;
                b.rot_z += b.rvz * dt;
                continue;
            }
            // In flight: integrate world_y under gravity + tumble.
            b.vy += GRAVITY * dt;
            b.world_y += b.vy * dt;
            b.rot_x += b.rvx * dt;
            b.rot_y += b.rvy * dt;
            b.rot_z += b.rvz * dt;
            // Collision against the table plane *or* the top of any
            // already-rested bone whose 2D footprint we overlap. The rest
            // height is half-thickness above whichever surface we hit so
            // the bone ends up lying flat on its widest face — like a
            // tablet that just slapped down — and the rotation snaps to
            // flat on landing so the half-thickness is the actual bottom
            // of the box.
            let half_thickness = b.extents[1] * 0.5;
            // Find the highest landed-bone surface this bone is about to
            // land on. We scan in `self.bones` directly because we hold
            // `&mut self.bones` here — but Rust borrow rules mean we can't
            // take an immutable reference to the same vec while mutating
            // an item, so the scan and the mutation are split: collect the
            // highest contact y first, then mutate `b` after.
            //
            // (Pulled out into its own scope below the loop body for
            // clarity.)
            let support_y = highest_support_below(&landed_snapshot, b, TABLE_Y);
            if b.world_y - half_thickness <= support_y {
                b.world_y = support_y + half_thickness;
                b.vy = 0.0;
                // Snap pitch + roll to zero so the bone rests flat. Yaw is
                // preserved so each bone keeps its own facing.
                b.rot_x = 0.0;
                b.rot_z = 0.0;
                // Bleed off the spin axes that would visibly wobble a flat
                // bone; leave a tiny yaw drift for life.
                b.rvx = 0.0;
                b.rvz = 0.0;
                b.rvy *= 0.4;
                b.rest_remaining = Some(REST_LIFETIME);
            }
        }
        self.bones
            .retain(|b| b.rest_remaining.map_or(true, |r| r > 0.0));
    }

    /// Drop every bone immediately (cascade ended). Also flushes any
    /// pending spawn queue so a fast-skip cascade doesn't keep dribbling
    /// new bones into an already-cleared play space.
    pub fn clear(&mut self) {
        self.bones.clear();
        self.pending.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.bones.is_empty() || !self.pending.is_empty()
    }

    /// Build the per-frame placement list the renderer consumes.
    pub fn placements(&self) -> Vec<FallingBonePlacement> {
        self.bones
            .iter()
            .map(|b| {
                let alpha = match b.rest_remaining {
                    Some(r) => (r / REST_LIFETIME).clamp(0.0, 1.0),
                    None => 1.0,
                };
                FallingBonePlacement {
                    world_pos: [b.px, b.py, b.world_y],
                    extents: b.extents,
                    rotation: [b.rot_x, b.rot_y, b.rot_z],
                    kind: b.kind,
                    alpha,
                }
            })
            .collect()
    }
}
