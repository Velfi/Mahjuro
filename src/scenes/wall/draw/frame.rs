//! Full strategic Wall frame assembly.

use crate::game::wall_stats::selected_tile_details;
use crate::render::doc_tile_camera::wall_ledger_camera;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::scenes::{BackgroundId, DrawCtx};
use crate::ui::controller_hints::{HintStyle, push_screen_footer_hint, wall_ledger_footer_row};

use super::super::StrategicWallScene;
use super::super::data::{build_frame_context, groups_by_face};
use super::super::focus::{LedgerNav, push_back_button};
use super::super::layout::{WallLayout, read_boost, wall_layout};
use super::super::state::WallScreenState;
use super::detail::draw_wall_detail_panel;
use super::grid::{draw_grid_panel_chrome, draw_tile_ledger_grid};
use super::header::draw_wall_header;
use super::summary::draw_wall_summary_panel;
use super::tabs::draw_wall_tabs;

pub fn draw_strategic_frame(scene: &StrategicWallScene, ctx: DrawCtx<'_>) -> UiFrame {
    let w = ctx.layout.window_w;
    let h = ctx.layout.window_h;
    let jr = read_boost(w, h);
    let frame_ctx = build_frame_context(ctx.run, scene.mode, scene.screen.view);
    let groups = groups_by_face(&frame_ctx.ledger);
    let layout = wall_layout(w, h, jr);

    let mut frame = UiFrame::new();
    frame.background(BackgroundId::Black);
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::WALNUT_INK,
        user: 0,
    });

    frame.camera_override = Some(wall_ledger_camera(h));
    frame.scene_lighting.push_smooth(PointLight {
        pos: [w * 0.5, h * -0.10, h * 1.40],
        radius: h * 3.0,
        color: color::rgb(color::PARCHMENT),
        intensity: 1.2,
    });

    push_back_button(&mut frame, &scene.focus.tree, w, h);

    let mut texts = Vec::new();
    draw_wall_header(&mut frame, &mut texts, w, h, jr, &layout, &frame_ctx.stats);
    draw_wall_tabs(
        &mut frame,
        &mut texts,
        &layout,
        scene.screen.view,
        scene.focus.focus,
    );
    draw_wall_summary_panel(
        &mut frame,
        &mut texts,
        &layout,
        &frame_ctx.stats,
        scene.screen.view,
        scene.focus.focus,
    );

    draw_grid_panel_chrome(&mut frame, &layout);

    let mut placements = Vec::new();
    draw_main_panel(
        &mut frame,
        &mut texts,
        &mut placements,
        &layout,
        &scene.screen,
        &frame_ctx.stats,
        &groups,
        ctx.run,
        scene.focus.focus,
        h,
    );

    let group = groups
        .get(&(scene.screen.selected.suit, scene.screen.selected.rank))
        .copied();
    let rep_tile = group.and_then(|g| {
        g.copies
            .iter()
            .find(|c| !c.drawn)
            .or_else(|| g.copies.first())
            .map(|c| c.tile)
    });
    if let Some(details) = selected_tile_details(
        &frame_ctx.stats,
        scene.screen.selected,
        &ctx.run.tile_debuffs,
        group,
    ) {
        draw_wall_detail_panel(
            &mut frame,
            &mut texts,
            &mut placements,
            &layout,
            &details,
            ctx.run,
            rep_tile.as_ref(),
        );
    }

    push_screen_footer_hint(
        &mut frame,
        &ctx,
        wall_ledger_footer_row(ctx.input_mode),
        HintStyle::standard(w, h),
    );

    frame.texts(texts);
    if !placements.is_empty() {
        frame.showcase_tile_batch(placements);
    }

    frame
}

fn draw_main_panel(
    frame: &mut UiFrame,
    texts: &mut Vec<TextLabel>,
    placements: &mut Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
    layout: &WallLayout,
    screen: &WallScreenState,
    stats: &crate::game::wall_stats::WallStats,
    groups: &std::collections::HashMap<
        (crate::core::tile::Suit, u8),
        &crate::game::wall_ledger::WallLedgerFaceGroup,
    >,
    run: &crate::game::run::RunState,
    focus: Option<LedgerNav>,
    h: f32,
) {
    draw_tile_ledger_grid(
        frame, texts, placements, layout, screen, stats, groups, run, focus, h,
    );
}
