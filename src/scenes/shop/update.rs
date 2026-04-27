use super::*;
use crate::scenes::{MeldGuideScene, OverlayRequest, YakuJournalScene, options};

impl ShopScene {
    pub(super) fn pause_options_overlay_impl(&self) -> Option<&options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    pub(super) fn has_blocking_overlay_impl(&self) -> bool {
        self.pause_menu.paused || self.pack_celebration.is_some()
    }

    pub(super) fn update_impl(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let shop = GameEngine::read_shop(ctx.run);
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        self.particles.update(dt);
        // Advance bug orbit phases.
        for (i, phase) in self.bug_phases.iter_mut().enumerate() {
            *phase = (*phase + BUG_PARAMS[i].2 * dt) % std::f32::consts::TAU;
        }
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
                center,
                center,
                None,
                crate::core::scoring::StepKind::Gold,
                new_level as f32,
            );
            self.particles
                .emit(center.0, center.1, 24, [0.95, 0.78, 0.25, 1.0], 0.9);
        }

        // Drain relic activations and evict expired glows.
        for rid in GameEngine::drain_relic_activations(ctx.run) {
            self.relic_glow_starts.insert(rid, now);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::RelicActivated(rid));
        }
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Help action opens the Meld Guide as an overlay.
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
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::MeldGuide(
                MeldGuideScene::new(true),
            ))));
            return None;
        }

        // Tile-pack opening celebration - swallow all input until
        // the player dismisses it with Confirm or any click.
        if let Some(ref mut celeb) = self.pack_celebration {
            let has_input = ctx.actions.iter().any(|a| {
                matches!(
                    a,
                    UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
                )
            }) || !ctx.button_clicks.is_empty();

            match celeb.phase {
                CelebPhase::Closeup => {
                    // Wait for player click/confirm to tear open the pack.
                    if has_input {
                        celeb.phase = CelebPhase::Reveal;
                        celeb.started_at = Instant::now();
                        ctx.bus.push(crate::game::event_bus::GameEvent::PackOpened);
                    }
                }
                CelebPhase::Reveal => {
                    // Fire a sound event for each newly-revealed tile.
                    let n = celeb.tiles.len();
                    while celeb.revealed_count < n
                        && celeb.tile_progress(celeb.revealed_count) > 0.0
                    {
                        ctx.bus
                            .push(crate::game::event_bus::GameEvent::PackTileRevealed);
                        celeb.revealed_count += 1;
                    }
                    let dominated = celeb.fully_settled() || celeb.dismissed;
                    if dominated && has_input {
                        self.pack_celebration = None;
                        return None;
                    }
                }
            }
            return None;
        }

        // Pause menu handling.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            // Drain a meld guide request from the pause menu.
            if self.pause_menu.take_meld_guide_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::MeldGuide(
                    MeldGuideScene::new(true),
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
                Some(ShopFocus::from_hit(hit))
            } else {
                focus_target_at_cursor(&focus_rects, cx, cy)
            };
            self.focus = new_focus;
        }

        let current_focus_rect = self.focus.and_then(|t| {
            focus_rects
                .iter()
                .find_map(|(t2, r)| (*t2 == t).then_some(*r))
        });

        for &a in ctx.actions {
            let dir: Option<FocusDir> = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
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
                if let Some(rect) = current_focus_rect
                    && let Some(next) = pick_neighbor(rect, dir, &focus_rects)
                {
                    self.focus = Some(next);
                }
                continue;
            }

            // LB / `[` sells the focused owned relic or consumable. RB keeps
            // the legacy relic reordering affordance for Mirror Tile setups.
            if matches!(a, UiAction::NavigateHudNext | UiAction::NavigateHudPrev) {
                if matches!(a, UiAction::NavigateHudPrev) {
                    match self.focus {
                        Some(ShopFocus::Relic(i)) => {
                            let n_for_sale = self.items.len();
                            if i >= n_for_sale {
                                let owned_idx = i - n_for_sale;
                                self.apply_sell_action(
                                    ShopAction::SellRelic(owned_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
                            }
                            continue;
                        }
                        Some(ShopFocus::Ribbon(i)) => {
                            if let Some(inv_idx) =
                                owned_ribbon_inventory_index(i, &self.zodiac_items, &shop)
                            {
                                self.apply_sell_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
                            }
                            continue;
                        }
                        Some(ShopFocus::Talisman(i)) => {
                            if let Some(inv_idx) =
                                owned_talisman_inventory_index(i, &self.talisman_items, &shop)
                            {
                                self.apply_sell_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
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

            // Controller/keyboard Confirm on an owned item starts a "hold to
            // drag" flow. In cursor mode the normal immediate action fires.
            if matches!(a, UiAction::Confirm) {
                if ctx.input_mode != InputMode::Cursor
                    && let Some(src) = drag_source_from_focus(
                        self.focus,
                        self.items.len(),
                        &self.zodiac_items,
                        &self.talisman_items,
                        &shop,
                    )
                {
                    self.held_item_drag = Some(src);
                    continue;
                }
                if let Some(focus) = self.focus {
                    if matches!(focus, ShopFocus::NextRound) {
                        return Some(self.continue_scene(ctx.run));
                    }
                    if matches!(focus, ShopFocus::Reroll)
                        && self.mode == ShopMode::Standard
                        && shop.gold >= self.reroll_cost as i32
                    {
                        self.reroll(ctx.run);
                        continue;
                    }
                    if let Some(hit) = focus.to_hit() {
                        if let Some(action) = shop_action_for_hit(
                            hit,
                            &self.items,
                            &self.zodiac_items,
                            &self.talisman_items,
                            &shop,
                        ) {
                            let result = apply_shop_action(
                                action,
                                &mut self.items,
                                &mut self.zodiac_items,
                                &mut self.talisman_items,
                                &mut self.pack_items,
                                ctx.run,
                                ctx.bus,
                            );
                            self.handle_shop_action_result(
                                result,
                                ctx.cursor_pos,
                                ctx.bus,
                                ctx.overlay_request,
                            );
                        } else if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                                super::Scene::YakuJournal(YakuJournalScene::new()),
                            )));
                            return None;
                        }
                    }
                }
                continue;
            }

            if matches!(a, UiAction::ConfirmRelease) {
                if let Some(drag) = self.held_item_drag.take()
                    && matches!(self.focus, Some(ShopFocus::SellTray))
                {
                    self.apply_sell_action(
                        drag.sell_action(),
                        ctx.run,
                        ctx.bus,
                        ctx.cursor_pos,
                        ctx.overlay_request,
                    );
                }
                continue;
            }

            if matches!(a, UiAction::Cancel) {
                self.held_item_drag = None;
                self.mouse_drag = None;
                self.focus = None;
                continue;
            }
        }

        for a in ctx.actions {
            if matches!(a, UiAction::CommitDiscard) {
                return Some(self.continue_scene(ctx.run));
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
                );
                return None;
            }
            if cid == SHOP_NEXT_ROUND_ID {
                return Some(self.continue_scene(ctx.run));
            }
            if cid == SHOP_REROLL_ID
                && self.mode == ShopMode::Standard
                && shop.gold >= self.reroll_cost as i32
            {
                self.reroll(ctx.run);
                return None;
            }
        }

        // Mouse drag-to-sell drop: injected by main.rs when the mouse button is
        // released over the sell tray after a drag that started on an owned item.
        for &cid in ctx.button_clicks {
            if cid != SHOP_DRAG_DROP_ID {
                continue;
            }
            if let Some(drag) = self.mouse_drag.take() {
                self.apply_sell_action(
                    drag.sell_action(),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                );
            }
            return None;
        }

        // 3D-hit dispatcher: route the action based on the renderer pick.
        for &cid in ctx.button_clicks {
            if cid != SHOP_3D_HIT_ID {
                continue;
            }
            let Some(hit) = ctx.picked_shop_object else {
                continue;
            };
            if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                    super::Scene::YakuJournal(YakuJournalScene::new()),
                )));
                return None;
            }
            if matches!(hit, ShopHit::Dish(id) if id == PICK_LEAVE_PROP) {
                return Some(self.continue_scene(ctx.run));
            }
            if matches!(hit, ShopHit::Dish(id) if id == PICK_REROLL_PROP) {
                if self.mode == ShopMode::Standard && shop.gold >= self.reroll_cost as i32 {
                    self.reroll(ctx.run);
                }
                return None;
            }
            if matches!(hit, ShopHit::Dish(id) if id == PICK_SELL_TRAY) {
                let sell_action = self.mouse_drag.take().map(|d| d.sell_action()).or_else(|| {
                    focused_sell_action(
                        self.focus,
                        self.items.len(),
                        &self.zodiac_items,
                        &self.talisman_items,
                        &shop,
                    )
                });
                if let Some(action) = sell_action {
                    self.apply_sell_action(
                        action,
                        ctx.run,
                        ctx.bus,
                        ctx.cursor_pos,
                        ctx.overlay_request,
                    );
                }
                return None;
            }
            self.mouse_drag = drag_source_from_hit(
                hit,
                self.items.len(),
                &self.zodiac_items,
                &self.talisman_items,
                &shop,
            );
            if let Some(action) = shop_action_for_hit(
                hit,
                &self.items,
                &self.zodiac_items,
                &self.talisman_items,
                &shop,
            ) {
                let result = apply_shop_action(
                    action,
                    &mut self.items,
                    &mut self.zodiac_items,
                    &mut self.talisman_items,
                    &mut self.pack_items,
                    ctx.run,
                    ctx.bus,
                );
                self.handle_shop_action_result(
                    result,
                    ctx.cursor_pos,
                    ctx.bus,
                    ctx.overlay_request,
                );
            }
            return None;
        }

        None
    }
}
