use super::*;

/// Pre-rasterized text label uploaded to the GPU as a texture + bind group.
/// Built per-frame in render() during the text-label pre-rasterize pass.
/// `bind_group` may be a clone of an entry in `text_label_cache`. When it is,
/// `_tex` is None (the texture is owned by the cache). For non-cacheable
/// marquee labels, we own the texture here so it stays alive until the frame
/// command buffer is submitted.
pub(super) struct TextDraw {
    pub inst_buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub scissor_rect: Option<[f32; 4]>,
    #[allow(dead_code)]
    pub _tex: Option<wgpu::Texture>,
}

/// One unit of work in the ordered render-op list. The cmd-walk in
/// `render()` builds a parallel ordered list of these; the encoder loop
/// later dispatches each to the appropriate pipeline / pass.
pub(super) enum RenderOp {
    /// Per-draw GPU instance buffer index into `ProcessOpCtx::bg_inst_buffers`.
    Background {
        id: BackgroundId,
        buf_idx: usize,
    },
    Starfield,
    EmberDrift,
    GoldenDust,
    MoonlitWater,
    SunlitWater,
    ShootingStarCascade,
    QuadBatch {
        buf_idx: usize,
        count: u32,
    },
    /// Screen-space quad that depth-tests against the scene depth buffer.
    DepthQuadBatch {
        buf_idx: usize,
        count: u32,
    },
    /// Post-tonemap tooltip / inspect panels (`DrawCmd::OverlayQuad`).
    OverlayQuadBatch {
        buf_idx: usize,
        count: u32,
    },
    GradientQuadBatch {
        buf_idx: usize,
        count: u32,
    },
    SquircleQuadBatch {
        buf_idx: usize,
        count: u32,
    },
    FlameBatch {
        buf_idx: usize,
        count: u32,
    },
    TextDraw(usize),
    TileFaceQuad(usize),
    ImageQuad(usize),
    /// Imported shop room (`shop.glb`), drawn like showcase tiles with identity model.
    ShopEnvironment,
    /// Pick-blind hallway (`hallway.glb`).
    HallwayEnvironment,
    /// Post-ordeal staircase (`staircase.glb`).
    StaircaseEnvironment,
    /// Archive room (`archive.glb`).
    ArchiveEnvironment,
    /// Main-menu waterfront (`main_menu.glb`).
    MainMenuEnvironment,
    /// Gameplay table room (`gameplay.glb`).
    GameplayEnvironment,
    /// Marker: start a new Pass A subpass with depth cleared (HDR color unchanged).
    /// Emitted from [`crate::draw_cmd::DrawCmd::ClearSceneDepth`]. Never dispatched
    /// through [`super::process_op::WgpuRenderer::process_op`].
    ClearSceneDepth,
    // Skeuomorphic gameplay HUD (phase 1).
    ShowcaseTileBatch(usize), // index into `showcase_tile_batches`
    Object3dBatch {
        start: usize,
        end: usize,
    }, // range into `object3d_draw_list`
}

/// Discriminator for the Object3d pre-pass / dispatch loop. Each kind
/// that gets drawn through the lit-mesh pipeline gets one variant here.
/// Keeping this as an enum (rather than raw u8 ids) means the compiler
/// catches any collision or missing dispatch arm.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DrawKind {
    YakuTablet,
    WoodTablet,
    Book,
    BookCover,
    Relic,
    BossIcon,
    Pack,
    Ribbon,
    Talisman,
    BugBody,
    BugWingL,
    BugWingR,
    BugWingBlurL,
    BugWingBlurR,
    Orb,
    Bowl,
    Mirror,
    TallyStickBase,
    TallyStickTip,
    ExtrudedGlyph,
    /// Standalone GLB prop drawn via the glTF PBR tile pipeline (yen coins).
    GltfCoin,
    Primitive(crate::primitive::MeshId),
}
