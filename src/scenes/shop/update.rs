use super::*;
use crate::scenes::journal_transition::{JournalDirection, JournalTransition};
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
        // Leave bell wobble — ease toward 1 while cursor/focus is on the bell
        // (same hit sources as draw; bypass `live_shop_hit` so Dish props like
        // `PICK_LEAVE_PROP` stay eligible).
        {
            let raw_hit = self
                .focus
                .and_then(|f| f.to_hit())
                .or(ctx.picked_shop_object);
            let leave_active = matches!(
                raw_hit,
                Some(ShopHit::Dish(id)) if id == PICK_LEAVE_PROP
            );
            let target = if leave_active { 1.0 } else { 0.0 };
            let k = 1.0 - (-14.0_f32 * dt).exp();
            self.leave_bell_hover_anim += (target - self.leave_bell_hover_anim) * k;
        }
        self.particles.update(dt);
        // Advance bug orbit phases.
        for (i, phase) in self.bug_phases.iter_mut().enumerate() {
            *phase = (*phase + BUG_PARAMS[i].2 * dt) % std::f32::consts::TAU;
        }

        // Yaku-Journal cover-open tween. Skip entirely when the
        // screenshot CLI has pinned the value with `--journal-open`.
        // When a click-to-open transition is in flight, the
        // `JournalTransition::open_progress` curve drives the open
        // amount directly, overriding focus. Once the full transition
        // window has elapsed, push `YakuJournalScene` and clear the
        // transition so the next frame is inside the journal.
        if let Some(p) = self.journal_transition_locked_at {
            // Re-anchor the synthetic transition each tick so wall-clock
            // drift can't move us past the locked progress fraction.
            // Read elapsed *after* re-anchoring so the captured frame
            // sees exactly the requested progress.
            let target_elapsed = JournalTransition::TOTAL_DUR * p;
            // Preserve the existing direction if any, default to
            // Opening — `set_journal_transition_for_screenshot`
            // assumes forward-direction captures.
            let dir = self
                .journal_transition
                .map(|t| t.dir)
                .unwrap_or(JournalDirection::Opening);
            self.journal_transition = Some(JournalTransition {
                start: now - std::time::Duration::from_secs_f32(target_elapsed),
                dir,
            });
        }
        // If we just resumed from `YakuJournalScene` (which was pushed
        // at the end of a forward transition), kick off the reverse
        // animation so the book closes back into its counter spot.
        // `journal_was_open` is the latch for that — set when we
        // pushed, cleared when the reverse starts.
        if self.journal_was_open && self.journal_transition.is_none() {
            self.journal_was_open = false;
            self.journal_transition = Some(JournalTransition {
                start: now,
                dir: JournalDirection::Closing,
            });
        }
        if let Some(t) = self.journal_transition {
            self.journal_open_amount = t.open_progress();
            self.journal_open_target = self.journal_open_amount;
            if self.journal_transition_locked_at.is_none() && t.done() {
                match t.dir {
                    JournalDirection::Opening => {
                        self.journal_transition = None;
                        self.journal_was_open = true;
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                            super::Scene::YakuJournal(YakuJournalScene::new()),
                        )));
                        return None;
                    }
                    JournalDirection::Closing => {
                        // Animation complete; clear and let the shop
                        // resume normally.
                        self.journal_transition = None;
                        self.journal_open_amount = 0.0;
                        self.journal_open_target = 0.0;
                    }
                }
            }
            // Hold here so the focus-driven branch below doesn't run
            // while the transition is in flight.
        } else if let Some(locked) = self.journal_open_lock {
            self.journal_open_amount = locked;
            self.journal_open_target = locked;
        } else {
            // Stay closed until click — no hover/focus peek; only `JournalTransition` opens.
            self.journal_open_target = 0.0;
            let rate = 6.0;
            let alpha = 1.0 - (-rate * dt).exp();
            self.journal_open_amount +=
                (self.journal_open_target - self.journal_open_amount) * alpha;
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
                        // Deferred from `apply_buy_action` while the celebration
                        // dimmer hid the HUD — snap to Leave now that it’s visible.
                        self.focus = Some(ShopFocus::NextRound);
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
                            self.apply_buy_action(
                                action,
                                ctx.run,
                                ctx.bus,
                                ctx.cursor_pos,
                                ctx.overlay_request,
                            );
                        } else if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                            // Click-to-open: start the journal transition
                            // animation. The scene push happens in
                            // `update_impl` once `JournalTransition::done()`
                            // returns true; until then, the cover-open +
                            // zoom animation plays in the shop.
                            if self.journal_transition.is_none() {
                                self.journal_transition = Some(JournalTransition {
                                    start: Instant::now(),
                                    dir: JournalDirection::Opening,
                                });
                            }
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
                self.focus = Some(ShopFocus::NextRound);
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
                if self.journal_transition.is_none() {
                    self.journal_transition = Some(JournalTransition {
                        start: Instant::now(),
                        dir: JournalDirection::Opening,
                    });
                }
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
                self.apply_buy_action(
                    action,
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                );
            }
            return None;
        }

        None
    }
}
