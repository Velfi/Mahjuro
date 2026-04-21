//! Generic 3D primitive: shape + material + optional decal.
//!
//! Replaces the bespoke-per-shape pattern where every new slab or column
//! needed a dedicated `Object3dKind` variant, a named `LitMeshGpu` field,
//! a named instance pool, a `DrawKind` variant, and a ~100-line match arm
//! in the renderer dispatch. Under this module a caller specifies a
//! [`MeshId`] (which mesh to draw) and a [`MaterialSpec`] (how to shade
//! it); `obj.color` is the base tint, honored consistently across every
//! shape. New shapes/materials are additive — add a [`MeshId`] variant
//! and register the mesh, nothing else needs touching.

use crate::render::lit_mesh::{MaterialKind, MaterialParams};

/// Which registered mesh to draw. Each variant maps to a [`LitMeshGpu`]
/// stashed in `WgpuRenderer::primitive_meshes` at construction time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshId {
    /// Unit cube, `-0.5..+0.5` on each axis, flat face normals.
    Cube,
    /// Unit cylinder, Y-up. Callers that expect a Z-up pose should
    /// compose with [`shape_orientation`] which inserts the
    /// mesh-Y-thickness-to-Z-up matrix automatically.
    Cylinder,
    /// Existing square dish mesh (thin slab with a lip).
    DiscSquare,
    /// Existing round dish mesh.
    DiscRound,
    /// Upright lacquered-wood slab with chain nubs (the former
    /// dedicated `Plaque` mesh).
    BeveledSlab,
    /// Hex-tower cabinet body, Z-up. Emits a second linked draw for
    /// [`MeshId::CabinetRails`] sharing the same model matrix.
    CabinetColumn,
    /// Companion brass rails that wrap a [`MeshId::CabinetColumn`].
    CabinetRails,
    /// Counter-end action-prop slab (former dedicated `ShopActionProp`
    /// mesh).
    ShopActionProp,
    /// Paper slab with an eyelet and chain nubs — former dedicated
    /// `Ofuda` mesh. Carries a title/rule calligraphy decal via
    /// `DecalLayout::TitleRule`.
    Ofuda,
}

/// Layout strategy for decal rasterization.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum DecalLayout {
    /// Auto-fit a single word-wrapped block of text into the decal
    /// texture. `target_short_edge` sets the shorter-axis resolution;
    /// the longer axis is derived from the host object's extents
    /// aspect ratio.
    Fit { target_short_edge: u32 },
    /// Two-line title over body copy. `title_height_frac` is the
    /// vertical fraction (0..1) reserved for the title.
    TitleRule {
        title_height_frac: f32,
        target_short_edge: u32,
    },
    /// Six adjacent label cells laid out horizontally; `text` is split
    /// on `\n` into exactly six entries (one per hex face).
    HexStrip,
    /// Caller-specified pixel dimensions. No auto-fit.
    Fixed { width: u32, height: u32 },
}

/// Preset palette for the three-pass engrave rasterizer.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum DecalPalette {
    /// Warm gold gilding over dark lacquer (plaques, cabinet faces).
    GoldGilded,
    /// Dark indigo ink on bone (yaku tablets).
    BoneInk,
    /// Dark ink on parchment (ofuda).
    ParchmentInk,
    /// Caller-supplied linear RGBA ink.
    MutedInk([f32; 4]),
}

/// Decal rasterization recipe.
#[derive(Clone, Debug)]
pub struct DecalSpec {
    pub text: String,
    pub palette: DecalPalette,
    pub layout: DecalLayout,
}

/// Material recipe for a primitive. Crucially does **not** store a
/// base color — `obj.color` is always the tint — so scenes cannot
/// accidentally set a tint that the material layer silently ignores.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MaterialSpec {
    pub kind: MaterialKind,
    pub specular_strength: f32,
    pub specular_power: f32,
    /// Additive emissive boost applied by the dispatch. 0.0 = off.
    /// Reserved for later migrations (Shrine/DoraPlinth/ShopLamp
    /// glow-target blending); phase-1 primitives pass 0.
    pub emissive: f32,
    pub decal: Option<DecalSpec>,
}

#[allow(dead_code)]
impl MaterialSpec {
    /// Upright thin lacquered-wood slab. Uses `LacqueredWoodFlat` so
    /// the vertex-displacing wood shader does not push the face
    /// through the slab thickness.
    pub fn lacquered_wood_flat() -> Self {
        Self {
            kind: MaterialKind::LacqueredWoodFlat,
            specular_strength: 0.55,
            specular_power: 96.0,
            emissive: 0.0,
            decal: None,
        }
    }

    /// Table-scale lacquered wood with vertex displacement (cabinet
    /// bodies, thick panels).
    pub fn lacquered_wood() -> Self {
        Self {
            kind: MaterialKind::LacqueredWood,
            specular_strength: 0.55,
            specular_power: 96.0,
            emissive: 0.0,
            decal: None,
        }
    }

    /// Polished brass conductor (shelf rails, display-case trim).
    pub fn brass() -> Self {
        Self {
            kind: MaterialKind::Brass,
            specular_strength: 0.85,
            specular_power: 128.0,
            emissive: 0.0,
            decal: None,
        }
    }

    /// Plain diffuse+specular dielectric.
    pub fn plain() -> Self {
        Self {
            kind: MaterialKind::Plain,
            specular_strength: 0.25,
            specular_power: 32.0,
            emissive: 0.0,
            decal: None,
        }
    }

    /// Polished metal conductor (coins, gold bars).
    pub fn metal() -> Self {
        Self {
            kind: MaterialKind::Metal,
            specular_strength: 0.9,
            specular_power: 196.0,
            emissive: 0.0,
            decal: None,
        }
    }

    /// Attach a decal to this material, returning the modified spec.
    pub fn with_decal(mut self, decal: DecalSpec) -> Self {
        self.decal = Some(decal);
        self
    }
}

/// Convenience: build a [`DecalSpec`] for the common gilded-gold
/// auto-fit layout used by every plaque in the game. Pairs with
/// [`MaterialSpec::lacquered_wood_flat`] to reproduce the legacy
/// `Object3dKind::Plaque { text, … }` ergonomics.
#[allow(dead_code)]
pub fn plaque_decal(text: impl Into<String>) -> DecalSpec {
    DecalSpec {
        text: text.into(),
        palette: DecalPalette::GoldGilded,
        layout: DecalLayout::Fit {
            target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
        },
    }
}

/// Translate a [`MaterialSpec`] + `obj.color` into the shader-facing
/// [`MaterialParams`]. This is the single place where `obj.color`
/// enters the lit-mesh pipeline for primitives, which fixes the
/// legacy bug where 15-of-25 Object3d kinds silently ignored the
/// `color` field.
///
/// When `silhouette` is true (locked-collection entries), the
/// material is forced to dark Plain regardless of the spec so the
/// slot still reads as the real shape without leaking texture/glow.
pub fn resolve_material(
    spec: &MaterialSpec,
    obj_color: [f32; 4],
    silhouette: bool,
) -> MaterialParams {
    if silhouette {
        return MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.04, 0.04, 0.05, obj_color[3]],
            specular_strength: 0.0,
            specular_power: 1.0,
        };
    }
    MaterialParams {
        kind: spec.kind,
        base_color: obj_color,
        specular_strength: spec.specular_strength,
        specular_power: spec.specular_power,
    }
}

/// Per-shape mesh-frame orientation applied before the caller's
/// rotation. Most shapes are authored in the renderer's world frame
/// directly (identity); a few Y-up meshes need the standard
/// mesh-Y-thickness-to-Z-up composition.
pub fn shape_orientation(shape: MeshId) -> glam::Mat4 {
    match shape {
        MeshId::Cylinder | MeshId::DiscRound => {
            crate::render::table_transform::mesh_y_thickness_along_local_y_to_z_up()
        }
        _ => glam::Mat4::IDENTITY,
    }
}
