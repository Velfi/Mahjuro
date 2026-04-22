//! Unified scene-placement primitives.
//!
//! Every manually-placeable object in a scene is described by a [`Placement`]:
//! normalized screen fractions for horizontal/vertical position, a physical
//! lift in millimeters, and three rotation angles in degrees (Z→Y→X order).
//!
//! Scenes compose their placements into a struct (e.g. `ShopPositions`,
//! `GameplayPositions`). Arrange-mode discovers placements by name via the
//! [`ArrangeTarget`] trait, so a single generic `apply_arrange` handler works
//! for every scene without bespoke match statements.
//!
//! ## Responsiveness
//!
//! Every placement uses the same coordinate system:
//!
//! - `nx`, `ny` are window fractions (0 = left/top, 1 = right/bottom). An
//!   object at `nx = 0.5` stays centered as the window resizes.
//! - `lift_mm` is physical millimeters above the felt, converted to world
//!   units through [`crate::ui::layout::LayoutResult::mm`] so physical object
//!   sizes stay consistent across resolutions.
//! - Rotations are coordinate-free degrees, composed Z→Y→X.
//!
//! Anchor-relative placements (hand strip, yaku tablet row, action-bar-
//! relative bowl/mirror, score-panel-relative plaque/coin pile) use the same
//! units — the scene interprets their `nx`/`ny` as fractional *offsets*
//! against a Cassowary-derived anchor rather than absolute screen positions.

use serde::{Deserialize, Serialize};

/// A manually-placeable object's position and rotation.
///
/// `nx` / `ny` are normalized window fractions (0–1, may go outside for
/// off-screen placements). `lift_mm` is physical millimeters above the felt.
/// Rotations are degrees; the renderer applies them in **Z → Y → X** order
/// (matches the existing hand-strip / yaku-tablet convention).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    /// 0 = left edge, 1 = right edge.
    #[serde(default)]
    pub nx: f32,
    /// 0 = top edge, 1 = bottom edge.
    #[serde(default)]
    pub ny: f32,
    /// Physical height above the felt, in millimeters. Converted to world
    /// units via `layout.mm()` so the physical scale stays constant.
    #[serde(default)]
    pub lift_mm: f32,
    /// Rotation around world X (pitch — tilt toward/away from camera), degrees.
    #[serde(default)]
    pub rx_deg: f32,
    /// Rotation around world Y (yaw — fan left/right), degrees.
    #[serde(default)]
    pub ry_deg: f32,
    /// Rotation around world Z (roll — spin on table plane), degrees.
    #[serde(default)]
    pub rz_deg: f32,
}

/// Accumulated arrange-mode delta applied to a `Placement` per axis.
/// `dnx` / `dny` are normalized fractions (pixel delta ÷ window size);
/// the rotation and lift deltas are absolute in their respective units.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArrangeDelta {
    pub dnx: f32,
    pub dny: f32,
    pub d_lift_mm: f32,
    pub d_rx_deg: f32,
    pub d_ry_deg: f32,
    pub d_rz_deg: f32,
}

impl Placement {
    /// Construct a placement at the given screen fraction and lift, with
    /// zero rotation.
    pub const fn at(nx: f32, ny: f32, lift_mm: f32) -> Self {
        Self {
            nx,
            ny,
            lift_mm,
            rx_deg: 0.0,
            ry_deg: 0.0,
            rz_deg: 0.0,
        }
    }

    /// True if every field is finite. A malformed JSON override could
    /// deserialize `NaN` or `Infinity` and then silently corrupt the scene;
    /// callers should reject non-finite placements at load time.
    pub fn is_finite(&self) -> bool {
        self.nx.is_finite()
            && self.ny.is_finite()
            && self.lift_mm.is_finite()
            && self.rx_deg.is_finite()
            && self.ry_deg.is_finite()
            && self.rz_deg.is_finite()
    }

    /// Apply an accumulated arrange-mode delta to this placement.
    #[inline]
    pub fn apply_delta(&mut self, delta: ArrangeDelta) {
        let ArrangeDelta {
            dnx,
            dny,
            d_lift_mm,
            d_rx_deg,
            d_ry_deg,
            d_rz_deg,
        } = delta;
        self.nx += dnx;
        self.ny += dny;
        self.lift_mm += d_lift_mm;
        self.rx_deg += d_rx_deg;
        self.ry_deg += d_ry_deg;
        self.rz_deg += d_rz_deg;
    }
}

/// Anchor derived from a [`Placement`] for constructing an `Object3d` that
/// will be auto-rotated by the renderer via `committed_arrange_rotations`.
///
/// This is the safe way to wire a scene `Placement` into an `Object3d`: it
/// folds the placement's `nx`/`ny`/`lift_mm` into the draw-site anchor and
/// leaves the rotation equal to `base_rotation` — the renderer composes the
/// placement's `rx_deg`/`ry_deg`/`rz_deg` on top of that via
/// `apply_arrange_override`, so the construction site MUST NOT read those
/// fields itself. Writing them into the rotation matrix here is exactly the
/// double-apply bug this type prevents.
///
/// Construction sites build an `Object3d` like so:
///
/// ```ignore
/// let anchor = PlacementAnchor::new(
///     [base_px, base_py, base_lift],       // draw-site anchor
///     base_rotation,                        // rotation that is NOT in the placement
///     &self.positions.my_thing,
///     "scene.my_thing",
///     layout,
/// );
/// Object3d {
///     pos: anchor.pos,
///     rotation: anchor.rotation,
///     arrange_name: Some(anchor.arrange_name),
///     // ...
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct PlacementAnchor {
    pub pos: [f32; 3],
    pub rotation: glam::Mat4,
    pub arrange_name: &'static str,
}

impl PlacementAnchor {
    /// Build a `PlacementAnchor` by folding a scene `Placement` into a
    /// draw-site anchor. See the type docs for contract details.
    pub fn new(
        base_pos: [f32; 3],
        base_rotation: glam::Mat4,
        placement: &Placement,
        arrange_name: &'static str,
        layout: &crate::ui::layout::LayoutResult,
    ) -> Self {
        Self {
            pos: [
                base_pos[0] + placement.nx * layout.window_w,
                base_pos[1] + placement.ny * layout.window_h,
                base_pos[2] + layout.mm(placement.lift_mm),
            ],
            rotation: base_rotation,
            arrange_name,
        }
    }
}

/// Staged arrange-mode delta for the currently-selected object (or group).
/// Passed to scenes via `DrawCtx::arrange_preview` so that non-`Object3d`
/// draws (wind emitters, particle sources, etc.) can apply the same live
/// preview that `apply_arrange_override` provides for mesh-backed placements.
///
/// Unlike the renderer's `DebugArrangeOverride` (which is in mixed pixel /
/// world units), these deltas are pre-normalised — ready to fold into a
/// `Placement` via [`Placement::apply_delta`].
#[derive(Clone, Debug)]
pub struct ArrangePreview {
    /// Selected leaf or group name (canonical dotted path).
    pub name: String,
    /// Normalised X delta (pixel delta ÷ window width).
    pub dnx: f32,
    /// Normalised Y delta (pixel delta ÷ window height).
    pub dny: f32,
    /// Lift delta in millimetres.
    pub d_lift_mm: f32,
    pub d_rx_deg: f32,
    pub d_ry_deg: f32,
    pub d_rz_deg: f32,
}

impl ArrangePreview {
    /// If `leaf` is the preview's target — either directly, or as a descendant
    /// of a selected group in `hierarchy` — return a copy of `base` with the
    /// staged deltas folded in. Otherwise return `base` unchanged.
    pub fn applied_to(&self, hierarchy: &'static [Node], leaf: &str, base: Placement) -> Placement {
        let affected = expand_name(hierarchy, &self.name);
        if !affected.contains(&leaf) {
            return base;
        }
        let mut p = base;
        p.apply_delta(ArrangeDelta {
            dnx: self.dnx,
            dny: self.dny,
            d_lift_mm: self.d_lift_mm,
            d_rx_deg: self.d_rx_deg,
            d_ry_deg: self.d_ry_deg,
            d_rz_deg: self.d_rz_deg,
        });
        p
    }
}

// ── Hierarchy ─────────────────────────────────────────────────────────────────

/// One entry in a scene's [`ArrangeTarget`] hierarchy. Either a leaf pointing
/// at a single placement, or a group that contains children.
///
/// Groups are useful both for *display* (a tree in the arrange-mode picker)
/// and for *batch nudging* (select `"shop.for_sale"` and every child column
/// gets the same delta).
#[derive(Clone, Debug)]
pub enum Node {
    /// Single placement. `name` is the canonical dotted path (e.g.
    /// `"shop.counter"`); `label` is the human-readable display text.
    Leaf {
        name: &'static str,
        label: &'static str,
    },
    /// Parent group. Selecting a group applies deltas to every descendant
    /// leaf. `name` is the dotted path (e.g. `"shop.for_sale"`).
    Group {
        name: &'static str,
        label: &'static str,
        children: &'static [Node],
    },
}

impl Node {
    pub fn name(&self) -> &'static str {
        match self {
            Node::Leaf { name, .. } | Node::Group { name, .. } => name,
        }
    }
}

/// Arrange-mode discovery trait: each scene's `*Positions` struct implements
/// this to expose its placements by name, letting a single generic
/// [`apply_arrange`] handler work uniformly across scenes.
///
/// # Names
///
/// Names are stable dotted paths (`"shop.counter"`, `"gameplay.hand_strip"`)
/// used as JSON keys, arrange-mode pickables, and display labels. There is
/// no alias layer — every caller uses the canonical path returned by
/// [`Self::hierarchy`].
pub trait ArrangeTarget {
    /// Return a mutable reference to the placement registered under `name`.
    /// `name` must be a canonical dotted path from [`Self::hierarchy`].
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement>;

    /// Read-only counterpart to [`Self::placement_mut`]. Used by arrange-mode
    /// HUD / logging to display current resolved coordinates alongside staged
    /// deltas.
    fn placement(&self, name: &str) -> Option<&Placement>;

    /// Return the hierarchical tree of placements this target exposes.
    /// Used by the arrange-mode picker to build a browsable tree.
    ///
    /// Default: empty tree (target not browseable).
    fn hierarchy(&self) -> &'static [Node] {
        &[]
    }
}

/// Collect every leaf name that is `name` itself, or a descendant of the
/// group named `name`. Returns an empty vec if `name` is not in the tree.
pub fn expand_name(hierarchy: &'static [Node], name: &str) -> Vec<&'static str> {
    fn walk(nodes: &'static [Node], target: &str, found: &mut Vec<&'static str>) -> bool {
        for n in nodes {
            if n.name() == target {
                collect_leaves(n, found);
                return true;
            }
            if let Node::Group { children, .. } = n
                && walk(children, target, found)
            {
                return true;
            }
        }
        false
    }
    fn collect_leaves(n: &'static Node, out: &mut Vec<&'static str>) {
        match n {
            Node::Leaf { name, .. } => out.push(name),
            Node::Group { children, .. } => {
                for c in *children {
                    collect_leaves(c, out);
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(hierarchy, name, &mut found);
    found
}

/// Apply an arrange-mode delta to `target`.
///
/// - If `name` is a leaf, the single placement receives the delta.
/// - If `name` is a group in the hierarchy, every descendant leaf receives
///   the delta (so a single "move the whole shelf" action works).
///
/// `name` must be a canonical path registered in the target's hierarchy.
///
/// Returns `true` if at least one placement was updated.
pub fn apply_arrange<T: ArrangeTarget + ?Sized>(
    target: &mut T,
    name: &str,
    delta: ArrangeDelta,
) -> bool {
    let members = expand_name(target.hierarchy(), name);
    let mut any = false;
    for m in members {
        if let Some(p) = target.placement_mut(m) {
            p.apply_delta(delta);
            any = true;
        }
    }
    any
}

/// Reset the placements under `name` (leaf or group) to the values from a
/// freshly-constructed `T::default()`. Returns `true` if at least one
/// placement was updated.
pub fn reset_arrange<T: ArrangeTarget + Default>(target: &mut T, name: &str) -> bool {
    let members = expand_name(target.hierarchy(), name);
    let mut defaults = T::default();
    let mut any = false;
    for m in members {
        if let (Some(dst), Some(src)) = (target.placement_mut(m), defaults.placement_mut(m)) {
            *dst = *src;
            any = true;
        }
    }
    any
}

/// Flat list of every leaf name in a hierarchy, in document order. Used by
/// coverage tests and by the renderer's committed-rotation map builder.
pub fn all_leaf_names(hierarchy: &'static [Node]) -> Vec<&'static str> {
    fn walk(nodes: &'static [Node], out: &mut Vec<&'static str>) {
        for n in nodes {
            match n {
                Node::Leaf { name, .. } => out.push(name),
                Node::Group { children, .. } => walk(children, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(hierarchy, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn placement_apply_delta_sums_all_axes() {
        let mut p = Placement::at(0.5, 0.5, 10.0);
        p.apply_delta(ArrangeDelta {
            dnx: 0.01,
            dny: 0.02,
            d_lift_mm: 3.0,
            d_rx_deg: 1.0,
            d_ry_deg: 2.0,
            d_rz_deg: 4.0,
        });
        assert!(approx(p.nx, 0.51));
        assert!(approx(p.ny, 0.52));
        assert!(approx(p.lift_mm, 13.0));
        assert!(approx(p.rx_deg, 1.0));
        assert!(approx(p.ry_deg, 2.0));
        assert!(approx(p.rz_deg, 4.0));
    }

    #[test]
    fn placement_serde_defaults_missing_fields() {
        // Only nx set; other fields should default to 0.
        let json = r#"{"nx": 0.5}"#;
        let p: Placement = serde_json::from_str(json).expect("deserialize");
        assert!(approx(p.nx, 0.5));
        assert!(approx(p.ny, 0.0));
        assert!(approx(p.lift_mm, 0.0));
        assert!(approx(p.rx_deg, 0.0));
    }

    #[test]
    fn placement_is_finite_rejects_nan() {
        let mut p = Placement::at(0.5, 0.5, 10.0);
        assert!(p.is_finite());
        p.nx = f32::NAN;
        assert!(!p.is_finite());
        p.nx = 0.5;
        p.lift_mm = f32::INFINITY;
        assert!(!p.is_finite());
    }

    struct DummyScene {
        a: Placement,
        b: Placement,
    }

    const DUMMY_HIERARCHY: &[Node] = &[Node::Group {
        name: "all",
        label: "All",
        children: &[
            Node::Leaf {
                name: "a",
                label: "A",
            },
            Node::Leaf {
                name: "b",
                label: "B",
            },
        ],
    }];

    impl ArrangeTarget for DummyScene {
        fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
            match name {
                "a" => Some(&mut self.a),
                "b" => Some(&mut self.b),
                _ => None,
            }
        }
        fn placement(&self, name: &str) -> Option<&Placement> {
            match name {
                "a" => Some(&self.a),
                "b" => Some(&self.b),
                _ => None,
            }
        }
        fn hierarchy(&self) -> &'static [Node] {
            DUMMY_HIERARCHY
        }
    }

    #[test]
    fn apply_arrange_group_hits_all_members() {
        let mut s = DummyScene {
            a: Placement::default(),
            b: Placement::default(),
        };
        let ok = apply_arrange(
            &mut s,
            "all",
            ArrangeDelta {
                dnx: 0.01,
                dny: 0.02,
                d_lift_mm: 1.0,
                ..Default::default()
            },
        );
        assert!(ok);
        assert!(approx(s.a.nx, 0.01));
        assert!(approx(s.b.nx, 0.01));
        assert!(approx(s.a.lift_mm, 1.0));
    }

    #[test]
    fn apply_arrange_unknown_name_returns_false() {
        let mut s = DummyScene {
            a: Placement::default(),
            b: Placement::default(),
        };
        let ok = apply_arrange(
            &mut s,
            "zzz",
            ArrangeDelta {
                dnx: 0.1,
                dny: 0.1,
                ..Default::default()
            },
        );
        assert!(!ok);
    }

    #[test]
    fn all_leaf_names_flattens_tree() {
        let names = all_leaf_names(DUMMY_HIERARCHY);
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn expand_name_on_leaf_returns_self() {
        let names = expand_name(DUMMY_HIERARCHY, "a");
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn expand_name_on_group_returns_descendants() {
        let names = expand_name(DUMMY_HIERARCHY, "all");
        assert_eq!(names, vec!["a", "b"]);
    }
}
