use super::view::snap_focus_after_shop_purchase;
use super::*;
use crate::scenes::{
    GuideScene, OverlayRequest, Scene, ShopInspectPresenter, ShowcasePresenter, ShowcaseScene,
    WallLedgerScene, YakuJournalScene, options,
};

/// Player inventory row is hit-tested via 3D picks only; exclude its rects from
/// cursor-mode [`focus_target_at_cursor`] so empty padding does not steal focus.
#[inline]
fn shop_owned_inventory_focus(scene: &ShopScene, f: ShopFocus) -> bool {
    match f {
        ShopFocus::Relic(i) => i >= scene.items.len(),
        ShopFocus::Ribbon(i) => i >= scene.zodiac_items.len(),
        ShopFocus::Talisman(i) => i >= scene.talisman_items.len(),
        _ => false,
    }
}

impl ShopScene {
    pub(super) fn pause_options_overlay_impl(&self) -> Option<&options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    pub(super) fn has_blocking_overlay_impl(&self) -> bool {
        self.pause_menu.paused
    }

    /// Push [`ShowcasePresenter::ShopInspect`] when `focus` can be orbited (relic / ribbon / talisman / pack).
    pub(super) fn try_push_shop_inspect_overlay(
        &self,
        focus: ShopFocus,
        w: f32,
        h: f32,
        shop_rm: &crate::game::engine::ShopReadModel,
        overlay_request: &mut Option<OverlayRequest>,
    ) {
        if !super::shared::shop_focus_inspectable(focus) {
            return;
        }
        let env_h = self.drawn_room_gltf_height_scale.get();
        let Some(orbit) =
            super::view::shop_item_inspect_orbit_for_focus(self, w, h, env_h, shop_rm, focus)
        else {
            return;
        };
        *overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Showcase(
            ShowcaseScene::new(ShowcasePresenter::ShopInspect(ShopInspectPresenter::new(
                orbit,
            ))),
        ))));
    }

    /// Mouse pick dispatch for shop hits (3D props + screen-space UI buttons).
    pub(super) fn dispatch_shop_pick_from_hit(
        &mut self,
        hit: ShopHit,
        ctx: &mut UpdateCtx<'_>,
    ) -> SceneTransition {
        let shop = GameEngine::read_shop(ctx.run);
        let hit = live_shop_hit(
            hit,
            self,
            &self.items,
            &self.zodiac_items,
            &self.talisman_items,
            &self.pack_items,
            &shop,
        )?;
        if matches!(hit, ShopHit::Dish(id) if id == PICK_LEAVE_PROP) {
            return Some(self.continue_intent());
        }
        if matches!(hit, ShopHit::Dish(id) if id == PICK_RESTOCK_PROP) {
            if !self.restock_exit_active()
                && self.mode == ShopMode::Standard
                && ctx.run.can_afford_shop_restock(self.restock_cost)
            {
                self.restock(ctx.run, ctx.bus);
            }
            return None;
        }
        if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::YakuJournal(
                YakuJournalScene::new(),
            ))));
            return None;
        }
        if let Some(action) = shop_action_for_hit(
            hit,
            &self.items,
            &self.zodiac_items,
            &self.talisman_items,
            &shop,
        ) {
            self.apply_buy_action(
                action,
                ctx.run,
                ctx.bus,
                ctx.cursor_pos,
                ctx.overlay_request,
                (ctx.layout.window_w, ctx.layout.window_h),
            );
        } else {
            self.focus = Some(ShopFocus::from_hit(hit));
        }
        None
    }

    /// Advance [`ShopScene::age_secs`] while the storeroom is suspended under
    /// [`ShopInspectPresenter`] so stock bob keeps running on non-inspected items.
    pub fn tick_suspended_animation_clock(&mut self) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        self.tick_departing_stock(now, 1080.0);
        self.gltf_anims.prune_finished();
    }

    /// Move inspect focus between inspectable stock items while the shop is suspended
    /// under [`ShopInspectPresenter`]. Returns true when focus changed.
    pub(crate) fn inspect_cycle_focus(
        &mut self,
        dir: FocusDir,
        run: &crate::game::run::RunState,
    ) -> bool {
        let shop = GameEngine::read_shop(run);
        let focus_rects = self.last_focus_rects.borrow().clone();
        let inspect_rects: Vec<(ShopFocus, [f32; 4])> = focus_rects
            .into_iter()
            .filter(|(focus, _)| {
                super::shared::shop_focus_inspectable(*focus)
                    && focus
                        .to_hit()
                        .and_then(|hit| {
                            live_shop_hit(
                                hit,
                                self,
                                &self.items,
                                &self.zodiac_items,
                                &self.talisman_items,
                                &self.pack_items,
                                &shop,
                            )
                        })
                        .is_some()
            })
            .collect();
        if inspect_rects.is_empty() {
            return false;
        }
        let current_focus_rect = self.focus.and_then(|focus| {
            inspect_rects
                .iter()
                .find_map(|(candidate, rect)| (*candidate == focus).then_some(*rect))
        });
        if current_focus_rect.is_none() {
            self.focus = inspect_rects.first().map(|(focus, _)| *focus);
            return self.focus.is_some();
        }
        if let Some(cur) = self.focus
            && let Some(next) = super::focus::shop_inspect_nav_pick(
                &mut self.focus_nav,
                &inspect_rects,
                cur,
                dir,
            )
            && Some(next) != self.focus
        {
            self.focus = Some(next);
            return true;
        }
        false
    }

    /// Play the focused stock item's bespoke audio (Confirm / A), when a relic stinger exists.
    pub(crate) fn push_inspect_artifact_sound_if_present(
        &self,
        run: &crate::game::run::RunState,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        let Some(focus) = self.focus else {
            return;
        };
        let ShopFocus::Relic(idx) = focus else {
            return;
        };
        let shop = GameEngine::read_shop(run);
        let n_sale = self.items.len();
        let rid = if idx < n_sale {
            self.items.get(idx).map(|it| it.relic)
        } else {
            shop.owned_relics
                .get(idx.saturating_sub(n_sale))
                .copied()
        };
        let Some(rid) = rid else {
            return;
        };
        let slug = rid.asset_filename().trim_end_matches(".png");
        let path = format!("audio/relics/{slug}.ogg");
        if crate::asset_path::get(&path).is_some() {
            bus.push(crate::game::event_bus::GameEvent::PlayRelicStinger(rid));
        }
    }

    pub(super) fn update_impl(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let shop = GameEngine::read_shop(ctx.run);
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        self.tick_departing_stock(now, ctx.layout.window_h);
        let (stick_x, stick_y) = ctx.shop_storeroom_orbit_stick;
        let orbit_active = ctx.mouse_left_down
            || stick_x.abs() > 1e-4
            || stick_y.abs() > 1e-4;
        if orbit_active {
            // Subtle pre-inspect peek — much slower and tighter than item-inspect orbit.
            const ORBIT: f32 = 0.55;
            const SENS: f32 = 0.010;
            const YAW_LIM: f32 = 0.12;
            const PITCH_LIM: f32 = 0.06;
            let (mx, my) = ctx.shop_storeroom_orbit_drag_px;
            let mut sx = -stick_x;
            let mut sy = -stick_y;
            if ctx.mouse_left_down {
                sx += (mx * SENS).clamp(-1.0, 1.0);
                sy += (-my * SENS).clamp(-1.0, 1.0);
            }
            self.storeroom_orbit_yaw =
                (self.storeroom_orbit_yaw + sx * ORBIT * dt).clamp(-YAW_LIM, YAW_LIM);
            self.storeroom_orbit_pitch =
                (self.storeroom_orbit_pitch + sy * ORBIT * dt).clamp(-PITCH_LIM, PITCH_LIM);
        } else if self.storeroom_orbit_yaw.abs() > 1e-5 || self.storeroom_orbit_pitch.abs() > 1e-5 {
            let k = 1.0 - (-7.0_f32 * dt).exp();
            self.storeroom_orbit_yaw += (0.0 - self.storeroom_orbit_yaw) * k;
            self.storeroom_orbit_pitch += (0.0 - self.storeroom_orbit_pitch) * k;
            if self.storeroom_orbit_yaw.abs() < 1e-5 {
                self.storeroom_orbit_yaw = 0.0;
            }
            if self.storeroom_orbit_pitch.abs() < 1e-5 {
                self.storeroom_orbit_pitch = 0.0;
            }
        }
        self.gltf_anims.prune_finished();
        if std::mem::take(&mut ctx.run.pending_shop_focus_snap_after_celebration) {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            snap_focus_after_shop_purchase(self, self.focus, w, h, ctx.run);
        }
        // Leave bell wobble — ease toward 1 while cursor/focus is on the bell
        // (same hit sources as draw; bypass `live_shop_hit` so Dish props like
        // `PICK_LEAVE_PROP` stay eligible).
        {
            let raw_hit = self
                .focus
                .and_then(|f| f.to_hit())
                .or(ctx.picked_shop_object)
                .and_then(|h| {
                    live_shop_hit(
                        h,
                        self,
                        &self.items,
                        &self.zodiac_items,
                        &self.talisman_items,
                        &self.pack_items,
                        &shop,
                    )
                });
            let leave_active = matches!(
                raw_hit,
                Some(ShopHit::Dish(id)) if id == PICK_LEAVE_PROP
            );
            let target = if leave_active { 1.0 } else { 0.0 };
            let k = 1.0 - (-14.0_f32 * dt).exp();
            self.leave_bell_hover_anim += (target - self.leave_bell_hover_anim) * k;
        }
        self.particles.update(dt, None);
        self.score_popups.update(now);

        // Drain finished zodiac celebration -> spawn popup + particles.
        if let Some((yaku_name, new_level)) = GameEngine::take_finished_zodiac_celebration(ctx.run)
        {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            let label = format!("{yaku_name} Lvl.{new_level}");
            let center = (w * 0.5, h * 0.45);
            self.score_popups.spawn(
                label,
                crate::render::world_space::LayoutAnchorPx {
                    px: center.0,
                    py: center.1,
                    lift_z: crate::render::score_popups::TABLE_POPUP_LIFT_Z,
                },
                center,
                None,
                crate::core::scoring::StepKind::Yen,
                new_level as f32,
                crate::render::score_popups::PopupMotionTiming::shipping_default(),
            );
            self.particles.emit(
                center.0,
                center.1,
                24,
                crate::render::theme::color::RELIC_GOLD,
                0.9,
            );
        }

        // Drain relic activations and evict expired glows.
        for rid in GameEngine::drain_relic_activations(ctx.run) {
            self.relic_glow_starts.insert(rid, now);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::RelicActivated(rid));
        }
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Help action opens the Guide as an overlay.
        let mut open_guide = false;
        for &cid in ctx.button_clicks {
            if cid == SHOP_HELP_BADGE_ID {
                open_guide = true;
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Help) {
                open_guide = true;
            }
        }
        if open_guide {
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                GuideScene::with_page(GuideScene::ECONOMY_PAGE),
            ))));
            return None;
        }

        // Pause menu handling.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            if self.pause_menu.take_credits_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Credits(
                    crate::scenes::CreditsScene::overlay(),
                ))));
                return None;
            }
            return t;
        }

        // The shop's focus graph is rebuilt every draw frame from projected
        // screen rects of every focusable element. Reuse the previous frame's
        // snapshot here for cursor hit-testing and directional nav.
        let focus_rects = self.last_focus_rects.borrow().clone();

        // Cursor-mode sync: when the player is using the mouse, hover is focus.
        if ctx.input_mode == InputMode::Cursor {
            let (cx, cy) = ctx.cursor_pos;
            let new_focus = if let Some(hit) = ctx.picked_shop_object {
                live_shop_hit(
                    hit,
                    self,
                    &self.items,
                    &self.zodiac_items,
                    &self.talisman_items,
                    &self.pack_items,
                    &shop,
                )
                .map(ShopFocus::from_hit)
            } else {
                let cursor_rects: Vec<(ShopFocus, [f32; 4])> = focus_rects
                    .iter()
                    .copied()
                    .filter(|(f, _)| !shop_owned_inventory_focus(self, *f))
                    .collect();
                focus_target_at_cursor(&cursor_rects, cx, cy)
            };
            self.focus = new_focus;
        }

        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        self.tick_hold_anchors(now, ctx.run, &shop);

        self.try_complete_west_sell_hold(
            now,
            &shop,
            ctx.run,
            ctx.bus,
            ctx.cursor_pos,
            ctx.overlay_request,
            (w, h),
        );
        self.try_complete_buy_hold(
            now,
            &shop,
            ctx.run,
            ctx.bus,
            ctx.cursor_pos,
            ctx.overlay_request,
            (w, h),
        );

        for &a in ctx.actions {
            if matches!(a, UiAction::NorthFacePress) {
                if let Some(f) = self.focus {
                    self.try_push_shop_inspect_overlay(f, w, h, &shop, ctx.overlay_request);
                }
                continue;
            }
            if matches!(a, UiAction::WestFacePress) {
                if focused_sell_action(
                    self.focus,
                    self.items.len(),
                    &self.zodiac_items,
                    &self.talisman_items,
                    &shop,
                )
                .is_some()
                    && self.west_sell_hold_started.is_none()
                {
                    self.cancel_buy_hold(ctx.bus);
                    self.west_sell_hold_started = Some(crate::ui::prompt_hold_ring::begin_hold(
                        now,
                        ctx.bus,
                        self.sell_hold_valid_for(&shop),
                    ));
                }
                continue;
            }
            if matches!(a, UiAction::WestFaceRelease) {
                self.cancel_west_sell_hold(ctx.bus);
                continue;
            }
            if matches!(a, UiAction::ConfirmRelease) {
                self.cancel_buy_hold(ctx.bus);
                continue;
            }

            let dir: Option<FocusDir> = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
                // Moving focus abandons any in-progress hold-to-buy/sell.
                self.cancel_all_hold_prompts(ctx.bus);
                if self.focus.is_none() {
                    let seed = focus_rects
                        .iter()
                        .find_map(|(t, _)| matches!(t, ShopFocus::Relic(_)).then_some(*t))
                        .or_else(|| focus_rects.first().map(|(t, _)| *t));
                    if let Some(t) = seed {
                        self.focus = Some(t);
                    }
                    continue;
                }
                if let Some(cur) = self.focus
                    && let Some(next) = super::focus::shop_directional_pick(
                        &mut self.focus_nav,
                        &focus_rects,
                        cur,
                        dir,
                    )
                {
                    self.focus = Some(next);
                }
                continue;
            }

            // LB / `[` and RB / `]` swap focused owned relics (Mirror Tile order).
            if matches!(a, UiAction::NavigateHudNext | UiAction::NavigateHudPrev) {
                if let Some(ShopFocus::Relic(i)) = self.focus {
                    let n_for_sale = self.items.len();
                    if i >= n_for_sale {
                        let owned_idx = i - n_for_sale;
                        let action = if matches!(a, UiAction::NavigateHudNext) {
                            ShopAction::MoveRelicRight(owned_idx)
                        } else {
                            ShopAction::MoveRelicLeft(owned_idx)
                        };
                        let _result = apply_shop_action(
                            action,
                            &mut self.items,
                            &mut self.zodiac_items,
                            &mut self.talisman_items,
                            &mut self.pack_items,
                            ctx.run,
                            ctx.bus,
                        );
                        let shop = GameEngine::read_shop(ctx.run);
                        if matches!(a, UiAction::NavigateHudNext)
                            && owned_idx + 1 < shop.owned_relics.len()
                        {
                            self.focus = Some(ShopFocus::Relic(n_for_sale + owned_idx + 1));
                        } else if matches!(a, UiAction::NavigateHudPrev) && owned_idx > 0 {
                            self.focus = Some(ShopFocus::Relic(n_for_sale + owned_idx - 1));
                        }
                    }
                }
                continue;
            }

            // Controller/keyboard Confirm on shop controls (cursor mode uses immediate picks).
            if matches!(a, UiAction::Confirm) {
                if let Some(focus) = self.focus {
                    if matches!(focus, ShopFocus::NextRound) {
                        return Some(self.continue_intent());
                    }
                    if matches!(focus, ShopFocus::Restock)
                        && !self.restock_exit_active()
                        && self.mode == ShopMode::Standard
                        && ctx.run.can_afford_shop_restock(self.restock_cost)
                    {
                        self.restock(ctx.run, ctx.bus);
                        continue;
                    }
                    if matches!(focus, ShopFocus::WallHud) {
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                            Scene::WallLedger(WallLedgerScene::shop_preview()),
                        )));
                        return None;
                    }
                    if let Some(hit) = focus.to_hit() {
                        if shop_action_for_hit(
                            hit,
                            &self.items,
                            &self.zodiac_items,
                            &self.talisman_items,
                            &shop,
                        )
                        .is_some()
                        {
                            // Hold-to-buy: charging the timer commits the
                            // purchase (see `try_complete_buy_hold`); a quick
                            // tap-and-release is cancelled by `ConfirmRelease`.
                            if self.buy_hold_started.is_none() {
                                self.cancel_west_sell_hold(ctx.bus);
                                self.buy_hold_started = Some(
                                    crate::ui::prompt_hold_ring::begin_hold(
                                        now,
                                        ctx.bus,
                                        self.buy_hold_valid_for(ctx.run, &shop),
                                    ),
                                );
                            }
                        } else if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                                Scene::YakuJournal(YakuJournalScene::new()),
                            )));
                            return None;
                        }
                    }
                }
                continue;
            }

            if matches!(a, UiAction::Cancel) {
                self.cancel_all_hold_prompts(ctx.bus);
                self.focus = Some(ShopFocus::NextRound);
                continue;
            }
        }

        for a in ctx.actions {
            if matches!(a, UiAction::CommitDiscard) {
                return Some(self.continue_intent());
            }
        }
        for &cid in ctx.button_clicks {
            if (SHOP_SELL_RELIC_BASE..SHOP_SELL_RELIC_BASE + 64).contains(&cid) {
                let idx = (cid - SHOP_SELL_RELIC_BASE) as usize;
                self.apply_sell_action(
                    ShopAction::SellRelic(idx),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                    (w, h),
                );
                return None;
            }
            if (SHOP_SELL_CONSUMABLE_BASE..SHOP_SELL_CONSUMABLE_BASE + 32).contains(&cid) {
                let idx = (cid - SHOP_SELL_CONSUMABLE_BASE) as usize;
                self.apply_sell_action(
                    ShopAction::SellConsumable(idx),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                    (w, h),
                );
                return None;
            }
            if cid == SHOP_NEXT_ROUND_ID {
                return Some(self.continue_intent());
            }
            if cid == SHOP_RESTOCK_ID
                && !self.restock_exit_active()
                && self.mode == ShopMode::Standard
                && ctx.run.can_afford_shop_restock(self.restock_cost)
            {
                self.restock(ctx.run, ctx.bus);
                return None;
            }
        }

        // 3D-hit dispatcher: route the action based on the renderer pick.
        for &cid in ctx.button_clicks {
            if cid != SHOP_3D_HIT_ID {
                continue;
            }
            let Some(hit) = ctx.picked_shop_object else {
                continue;
            };
            return self.dispatch_shop_pick_from_hit(hit, &mut ctx);
        }

        None
    }
}
