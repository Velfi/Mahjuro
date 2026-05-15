//! Full-screen [`crate::scenes::Scene::Showcase`] overlay — one scene type for every flow below:
//! - **Celebrations**: [`TilePackPresenter`], [`ZodiacPresenter`], [`MetaLevelUpPresenter`]
//! - **Orbit inspect**: [`ShopInspectPresenter`] (storeroom), [`CollectionInspectPresenter`] (Archive)
//!
//! Presenters share [`ShowcaseRenderHints`](crate::render::draw_cmd::ShowcaseRenderHints) on [`UiFrame`].

mod collection_inspect;
mod meta_level_up;
mod shop_inspect;
mod tile_pack;
mod zodiac;

pub use collection_inspect::CollectionInspectPresenter;
pub use meta_level_up::MetaLevelUpPresenter;
pub use shop_inspect::ShopInspectPresenter;
pub use tile_pack::TilePackPresenter;
pub use zodiac::ZodiacPresenter;

use crate::render::draw_cmd::UiFrame;
use crate::ui::scene_layout::load_shop_positions;

use super::{DrawCtx, SceneBehavior, SceneTransition, UpdateCtx};

/// Which flow is running on the showcase overlay.
pub enum ShowcasePresenter {
    TilePack(TilePackPresenter),
    Zodiac(ZodiacPresenter),
    MetaLevelUp(MetaLevelUpPresenter),
    ShopInspect(ShopInspectPresenter),
    CollectionInspect(CollectionInspectPresenter),
}

impl ShowcasePresenter {
    pub fn wants_orbit_input(&self) -> bool {
        matches!(self, Self::ShopInspect(_) | Self::CollectionInspect(_))
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        match self {
            Self::TilePack(p) => p.update(ctx),
            Self::Zodiac(p) => p.update(ctx),
            Self::MetaLevelUp(p) => p.update(ctx),
            Self::ShopInspect(p) => p.update(ctx),
            Self::CollectionInspect(p) => p.update(ctx),
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        match self {
            Self::TilePack(p) => p.draw_frame(ctx),
            Self::Zodiac(p) => p.draw_frame(ctx),
            Self::MetaLevelUp(p) => p.draw_frame(ctx),
            Self::ShopInspect(p) => p.draw_frame(ctx),
            Self::CollectionInspect(p) => p.draw_frame(ctx),
        }
    }

    fn reload_shop_positions_if_tile_pack(&mut self) {
        if let Self::TilePack(p) = self {
            p.positions = load_shop_positions();
        }
    }
}

/// Same overlay stack as pack/zodiac celebrations; shop and collection inspect use this too.
pub struct ShowcaseScene {
    pub presenter: ShowcasePresenter,
}

impl ShowcaseScene {
    pub fn new(presenter: ShowcasePresenter) -> Self {
        Self { presenter }
    }

    pub fn wants_orbit_input(&self) -> bool {
        self.presenter.wants_orbit_input()
    }

    pub fn reload_shop_positions_from_disk(&mut self) {
        self.presenter.reload_shop_positions_if_tile_pack();
    }
}

impl SceneBehavior for ShowcaseScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.presenter.update(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.presenter.draw_frame(ctx)
    }

    fn has_blocking_overlay(&self) -> bool {
        true
    }
}
