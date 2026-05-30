//! Animated coins that fly into or out of the coin dish when gold changes.
//!
//! Gains: coins drop from above into the dish, landing with a bounce.
//! Losses: coins launch upward out of the dish and fade out.
//!
//! Each coin carries its own pixel-space anchor (pinned to the dish center
//! with random scatter), a world_y height that evolves under gravity, and a
//! yaw rotation for visual variety. The system outputs `CoinPlacement`
//! structs that merge directly into the existing coin batch.

use rand::RngExt;

// ── Physics constants ────────────────────────────────────────────────────

/// Gravity acceleration (world_y units / s²). Negative = downward.
const GRAVITY: f32 = -1800.0;

/// Height above the dish rim where incoming coins spawn.
const SPAWN_HEIGHT: f32 = 450.0;

/// Upward launch speed for coins leaving the dish.
const LAUNCH_SPEED: f32 = 600.0;

/// How long an outgoing (loss) coin lives before being removed (seconds).
const LOSS_LIFETIME: f32 = 0.65;

/// Seconds a landed coin lingers at rest before despawning.
const REST_DURATION: f32 = 0.35;

/// Horizontal scatter radius around the dish center (pixels).
const SCATTER: f32 = 70.0;

/// Stagger window: coins in a burst spawn over this many seconds so they
/// arrive as a stream, not a simultaneous clump.
const BURST_SPREAD: f32 = 0.30;

/// Coefficient of restitution for the landing bounce. Each bounce loses
/// this fraction of velocity; once the bounce height would be negligible
/// the coin snaps to rest.
const BOUNCE_FACTOR: f32 = 0.3;

/// Minimum |vy| below which a bouncing coin just snaps to rest.
const BOUNCE_THRESHOLD: f32 = 40.0;

// ── Per-coin state ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct FlyingCoin {
    /// Pixel-space anchor (maps to world xz via [`crate::world_space::pixel_to_world`]).
    px: f32,
    py: f32,
    /// Height above the table surface. Evolves under gravity.
    world_y: f32,
    /// Vertical velocity (world units / s).
    vy: f32,
    /// Yaw rotation in radians — spins slowly for visual variety.
    rot_y: f32,
    /// Angular velocity around Y (radians / s).
    rvy: f32,
    /// World-y of the surface this coin should land on (dish rim height
    /// plus half-thickness, so it comes to rest flush with the pile top).
    landing_y: f32,
    /// Coin geometry.
    radius: f32,
    thickness: f32,
    /// Direction: incoming (gain) or outgoing (loss).
    kind: CoinFlyKind,
    /// Seconds remaining before despawn. `None` while in flight (gains) or
    /// always counting down (losses).
    life_remaining: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoinFlyKind {
    /// Coin dropping into the dish (gold gained).
    Gain,
    /// Coin launching out of the dish (gold spent).
    Loss,
}

/// A coin waiting to spawn after a brief delay.
#[derive(Clone, Debug)]
struct PendingCoin {
    delay: f32,
    px: f32,
    py: f32,
    landing_y: f32,
    radius: f32,
    thickness: f32,
    kind: CoinFlyKind,
}

// ── System ───────────────────────────────────────────────────────────────

pub struct FlyingCoinSystem {
    coins: Vec<FlyingCoin>,
    pending: Vec<PendingCoin>,
}

impl Default for FlyingCoinSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FlyingCoinSystem {
    pub fn new() -> Self {
        Self {
            coins: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Spawn `count` coins dropping into the dish (gold gained).
    pub fn gain(
        &mut self,
        pile_cx: f32,
        pile_cy: f32,
        landing_y: f32,
        radius: f32,
        thickness: f32,
        count: usize,
    ) {
        let mut rng = rand::rng();
        for _ in 0..count {
            let dx = (rng.random::<f32>() - 0.5) * SCATTER * 2.0;
            let dy = (rng.random::<f32>() - 0.5) * SCATTER * 2.0;
            self.pending.push(PendingCoin {
                delay: rng.random::<f32>() * BURST_SPREAD,
                px: pile_cx + dx,
                py: pile_cy + dy,
                landing_y: landing_y + thickness * 0.5,
                radius,
                thickness,
                kind: CoinFlyKind::Gain,
            });
        }
    }

    /// Spawn `count` coins launching out of the dish (gold spent).
    pub fn lose(
        &mut self,
        pile_cx: f32,
        pile_cy: f32,
        start_y: f32,
        radius: f32,
        thickness: f32,
        count: usize,
    ) {
        let mut rng = rand::rng();
        for _ in 0..count {
            let dx = (rng.random::<f32>() - 0.5) * SCATTER * 2.0;
            let dy = (rng.random::<f32>() - 0.5) * SCATTER * 2.0;
            self.pending.push(PendingCoin {
                delay: rng.random::<f32>() * BURST_SPREAD,
                px: pile_cx + dx,
                py: pile_cy + dy,
                landing_y: start_y,
                radius,
                thickness,
                kind: CoinFlyKind::Loss,
            });
        }
    }

    /// Advance simulation by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        // Drip pending coins into the live pool.
        let mut i = self.pending.len();
        while i > 0 {
            i -= 1;
            self.pending[i].delay -= dt;
            if self.pending[i].delay <= 0.0 {
                let p = self.pending.swap_remove(i);
                self.spawn_one(p);
            }
        }

        for c in &mut self.coins {
            c.rot_y += c.rvy * dt;

            match c.kind {
                CoinFlyKind::Gain => {
                    if let Some(rest) = c.life_remaining.as_mut() {
                        // At rest — count down to despawn.
                        *rest -= dt;
                        continue;
                    }
                    // In flight: integrate gravity.
                    c.vy += GRAVITY * dt;
                    c.world_y += c.vy * dt;
                    // Landing check.
                    if c.world_y <= c.landing_y {
                        c.world_y = c.landing_y;
                        // Bounce or rest.
                        let rebound = (-c.vy * BOUNCE_FACTOR).abs();
                        if rebound < BOUNCE_THRESHOLD {
                            c.vy = 0.0;
                            c.life_remaining = Some(REST_DURATION);
                        } else {
                            c.vy = rebound;
                        }
                    }
                }
                CoinFlyKind::Loss => {
                    // Always counting down.
                    if let Some(rest) = c.life_remaining.as_mut() {
                        *rest -= dt;
                    }
                    c.vy += GRAVITY * dt;
                    c.world_y += c.vy * dt;
                }
            }
        }

        // Remove expired coins.
        self.coins
            .retain(|c| c.life_remaining.is_none_or(|r| r > 0.0));
    }

    fn spawn_one(&mut self, p: PendingCoin) {
        let mut rng = rand::rng();
        let rot_y = rng.random_range(-std::f32::consts::PI..std::f32::consts::PI);
        let rvy = (rng.random::<f32>() - 0.5) * 4.0;

        let (world_y, vy, life) = match p.kind {
            CoinFlyKind::Gain => {
                let h = SPAWN_HEIGHT + rng.random::<f32>() * 80.0;
                let v = -(rng.random::<f32>() * 30.0);
                (h, v, None)
            }
            CoinFlyKind::Loss => {
                let v = LAUNCH_SPEED + rng.random::<f32>() * 80.0;
                (p.landing_y, v, Some(LOSS_LIFETIME))
            }
        };

        self.coins.push(FlyingCoin {
            px: p.px,
            py: p.py,
            world_y,
            vy,
            rot_y,
            rvy,
            landing_y: p.landing_y,
            radius: p.radius,
            thickness: p.thickness,
            kind: p.kind,
            life_remaining: life,
        });
    }

    /// Build placements for the renderer. Merges directly into an Object3d batch.
    pub fn placements(&self) -> Vec<crate::draw_cmd::Object3d> {
        use crate::draw_cmd::{Object3d, Object3dKind};
        self.coins
            .iter()
            .map(|c| {
                let alpha = match (c.kind, c.life_remaining) {
                    // Losses fade out over their lifetime.
                    (CoinFlyKind::Loss, Some(r)) => (r / LOSS_LIFETIME).clamp(0.0, 1.0),
                    // Gains at rest fade out.
                    (CoinFlyKind::Gain, Some(r)) => (r / REST_DURATION).clamp(0.0, 1.0),
                    _ => 1.0,
                };
                Object3d {
                    pos: [c.px, c.py, c.world_y],
                    extents: [c.radius * 2.0, c.thickness, c.radius * 2.0],
                    rotation: [0.0, c.rot_y, 0.0],
                    color: [1.0, 1.0, 1.0, alpha],
                    kind: Object3dKind::Primitive {
                        shape: crate::primitive::MeshId::Coin,
                        material: crate::primitive::MaterialSpec::coin_glb(),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                }
            })
            .collect()
    }

    pub fn is_active(&self) -> bool {
        !self.coins.is_empty() || !self.pending.is_empty()
    }
}
