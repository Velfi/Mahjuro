/// One hit returned by `WgpuRenderer::pick_shop_object`. The renderer's pick
/// path tests against three categories: relic cuboids (RelicBatch), ribbons
/// (ZodiacBatch), and explicit dishes (DishExplicit). The shop scene further
/// partitions the relic/ribbon indices into for-sale vs owned by tracking
/// how many of each it pushed in the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShopHit {
    /// Index into the most recent flat list of `RelicPlacement`s pushed this
    /// frame (across all `RelicBatch` cmds).
    Relic(usize),
    /// Index into the most recent flat list of `ZodiacRibbonPlacement`s
    /// pushed this frame (across all `ZodiacBatch` cmds).
    Ribbon(usize),
    /// Index into the most recent flat list of `TalismanPlacement`s pushed
    /// this frame (across all `TalismanBatch` cmds).
    Talisman(usize),
    /// The auxiliary dish whose `pick_id` matched. The scene assigns ids
    /// when it pushes the dish (e.g. `1` for the relic dish, `2` for the
    /// coin dish).
    Dish(u32),
    /// Index into the most recent flat list of `TilePackPlacement`s pushed
    /// this frame (across all `TilePackBatch` cmds).
    TilePack(u32),
    /// shop.glb trimesh hit on `shop_spawn_relic_{slot:02}` — resolve via
    /// [`crate::scenes::shop::layout::live_shop_hit`] before using as a relic index.
    EnvSpawnSlot(usize),
    /// shop.glb trimesh hit on `shop_player_relic_{slot:02}` — inventory bar index.
    EnvInvSlot(usize),
    /// shop.glb trimesh hit on `shop_player_consumable_{ord:02}` consumable marker ordinal.
    EnvConsumableOrd(usize),
}

/// Reserved primitive pick ids on the diegetic main menu exterior scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuPick {
    Play,
    Options,
    Quit,
}

impl MainMenuPick {
    #[inline]
    pub fn from_pick_id(id: u32) -> Option<Self> {
        match id {
            240 => Some(Self::Play),
            241 => Some(Self::Options),
            242 => Some(Self::Quit),
            _ => None,
        }
    }
}

/// What 3D gameplay-scene object the cursor is over this frame.
///
/// Resolved by [`WgpuRenderer::pick_gameplay_object`] via per-class local
/// AABB raycasting against the previous frame's model matrices — the same
/// pattern as `pick_hand_tile` / `pick_shop_object`. The gameplay scene
/// uses this for hover state and the click-injection path uses it to
/// route mouse clicks to the right action without screen-space rect
/// projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameplayPick {
    /// Index into the most recent `YakuTabletBatch` (hover only — yaku
    /// tablets aren't clickable, just informational).
    YakuTablet(usize),
    /// Index into the most recent `WoodTabletBatch` — 0 = sort suit,
    /// 1 = sort rank, 2 = cash-in tablet when structure is committed.
    WoodTablet(usize),
    /// Leather-bound Yaku Journal book (same mesh as the shop).
    JournalBook,
    /// The discard bowl. Click target = commit the selected discard.
    DiscardBowl,
    /// The bronze mirror. Click target = play the selected hand.
    BronzeMirror,
    /// Main menu exterior: doorway, regulations sign, or bicycle hit proxy.
    MainMenu(MainMenuPick),
}
