use super::*;

impl ShopScene {
    pub(super) fn draw_frame_impl(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let shop = GameEngine::read_shop(ctx.run);
        let n_for_sale_zodiacs = self.zodiac_items.len();
        let n_for_sale_talismans = self.talisman_items.len();
        let n_owned_relics = shop.owned_relics.len();
        let n_owned_zodiacs = shop.owned_zodiacs.len();
        let n_owned_talismans = shop.owned_talismans.len();
        let layout = ShopLayout::build(
            ctx.layout,
            &self.positions,
            ShopInventoryCounts {
                n_for_sale: self.items.len(),
                n_for_sale_zodiacs,
                n_for_sale_talismans,
                n_owned_relics,
                n_owned_zodiacs,
                n_owned_talismans,
            },
        );

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        // Procedural mountain-haze wash. Additively composed onto the black
        // background, sits behind the 3D scene and above the volumetric
        // smoke curtain so the scene reads as "shop on a foggy mountain."
        frame.mountain_haze();
        frame.camera_override = Some(layout.camera);

        let (plaque_top_text, plaque_bot_text) = shop_plaque_lines(self, &shop);

        // ── Kiosk counter slab (thin wide dish — reads as a face-up surface) ─
        // center_pos is (pixel_x, pixel_y, lift_z); extents are (width_x, rim_z, depth_y).
        frame.object3d(Object3d {
            pos: [
                layout.counter_pixel_x,
                layout.counter_world_y + h * 0.5,
                layout.counter_extents[1] * 0.5,
            ],
            extents: layout.counter_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });

        // ── Foreground dishes (relic + talisman + ribbon trays + gold) ─
        frame.object3d(Object3d {
            pos: [
                layout.relic_dish_center_px.0,
                layout.relic_dish_center_px.1,
                layout.relic_dish_center_px.2 + layout.relic_dish_extents[1] * 0.5,
            ],
            extents: layout.relic_dish_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: Some(PICK_RELIC_DISH),
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.relic_dish"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.talisman_tray_center_px.0,
                layout.talisman_tray_center_px.1,
                layout.talisman_tray_center_px.2 + layout.talisman_tray_extents[1] * 0.5,
            ],
            extents: layout.talisman_tray_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.talisman_tray"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.ribbon_tray_center_px.0,
                layout.ribbon_tray_center_px.1,
                layout.ribbon_tray_center_px.2 + layout.ribbon_tray_extents[1] * 0.5,
            ],
            extents: layout.ribbon_tray_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.ribbon_tray"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.coin_dish_center_px.0,
                layout.coin_dish_center_px.1,
                layout.coin_dish_center_px.2 + layout.coin_dish_extents[1] * 0.5,
            ],
            extents: layout.coin_dish_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscRound,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: Some(PICK_COIN_DISH),
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });
        // Yaku Journal anchor — placed later (after `cam_rot` and
        // `hover` are in scope) as a wood action tablet. These
        // bindings stay here because downstream lighting code keys
        // point lights off `journal_cx/cy/cz`.
        let journal_cx = self.positions.book.nx * w;
        let journal_cy = self.positions.book.ny * h;
        let journal_cz = layout.mm(self.positions.book.lift_mm);

        // Tile packs — two flanking positions in column 2, on the counter.
        // Hidden while the pack-opening celebration is active: the celebration
        // draws its own large closeup pack centered on screen, and the 2D dim
        // quad can't depth-occlude the shelf packs behind it.
        if self.pack_celebration.is_none() {
            let ext = layout.pack_extents;
            let mut pack_objs: Vec<Object3d> = Vec::new();
            for (i, pack) in self.pack_items.iter().enumerate() {
                if i >= N_TILE_PACKS || pack.sold {
                    continue;
                }
                let (cx, cy, cz) = layout.pack_centers_px[i];
                pack_objs.push(Object3d {
                    // Center-lift so the pack sits on the counter (ext[2] is height).
                    pos: [cx, cy, cz + ext[2] * 0.5],
                    extents: ext,
                    rotation: glam::Mat4::IDENTITY,
                    color: pack.kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: pack.kind,
                        pick_id: Some(PICK_TILE_PACK_BASE + i as u32),
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
            if !pack_objs.is_empty() {
                frame.object3d_batch(pack_objs);
            }
        }

        // ── Relic batch: for-sale relics in column 1, then owned in tray.
        // The order matters: pick_shop_object
        // returns indices into a flat list, so we partition with the
        // for-sale slots first and the owned slots second.
        let mut relic_objects: Vec<Object3d> = Vec::new();
        let niche_base = layout.counter_extents[0] * 0.055;
        for (i, item) in self.items.iter().enumerate() {
            if i >= layout.niche_count {
                break;
            }
            let (px, py, wy) = layout.niche_centers_px[i];
            let half = relic_half_extents(item.relic, niche_base);
            let col = if item.sold {
                color::alpha(rarity_color(item.rarity), 0.35)
            } else {
                rarity_color(item.rarity)
            };
            relic_objects.push(Object3d {
                pos: [px, py, wy + half[2]], // lift center by half face-height (local Z)
                extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                rotation: rot_rx_rz_deg(SHOP_RELIC_LEAN_COUNTER, 0.0),
                color: col,
                kind: Object3dKind::Relic {
                    relic_id: item.relic,
                    glow: 0.0,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        let owned_base = layout.relic_dish_extents[0] * 0.15;
        for (i, &rid) in shop.owned_relics.iter().enumerate() {
            let (px, py, wy) = layout.owned_relic_pos(i);
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(Rarity::Common);
            let half = relic_half_extents(rid, owned_base);
            let (glow, wiggle_deg) = if let Some(start) = self.relic_glow_starts.get(&rid) {
                let age = Instant::now()
                    .saturating_duration_since(*start)
                    .as_secs_f32();
                let life = RELIC_GLOW_LIFETIME.as_secs_f32();
                if age >= life {
                    (0.0, 0.0)
                } else {
                    let t = (age / life).clamp(0.0, 1.0);
                    let attack_end = 0.12_f32;
                    let g = if t < attack_end {
                        (t / attack_end).clamp(0.0, 1.0)
                    } else {
                        let decay_t = (t - attack_end) / (1.0 - attack_end);
                        (1.0 - decay_t).max(0.0).powi(2)
                    };
                    (g, g * 12.0 * (age * 22.0).sin())
                }
            } else {
                (0.0, 0.0)
            };
            let wiggle = glam::Mat4::from_rotation_z(wiggle_deg.to_radians());
            relic_objects.push(Object3d {
                pos: [px, py, wy + half[2]], // lift center by half face-height (local Z)
                extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                rotation: wiggle * rot_rx_rz_deg(SHOP_RELIC_LEAN_INVENTORY, 0.0),
                color: rarity_color(rarity),
                kind: Object3dKind::Relic {
                    relic_id: rid,
                    glow,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        if !relic_objects.is_empty() {
            frame.object3d_batch(relic_objects);
        }

        // ── Consumable batches: zodiacs are silken ribbons (upper-right
        //    cabinet zone), talismans are jade octagonal tablets (lower-
        //    right cabinet zone, below the shelf divider). Each gets its
        //    own batch, pick path, and dedicated wall/tray positions.
        let mut consumable_objects: Vec<Object3d> = Vec::new();

        // For-sale zodiacs: upper-right cabinet wall.
        for (i, item) in self.zodiac_items.iter().enumerate() {
            if i >= layout.ribbon_count {
                break;
            }
            let (ax, ay, awy) = layout.ribbon_anchors_px[i];
            let mut col = consumable_color(item.consumable);
            if item.sold {
                col[3] = 0.30;
            }
            let w = layout.ribbon_width;
            consumable_objects.push(Object3d {
                pos: [ax, ay, awy],
                extents: [w, layout.ribbon_length, w * 0.15],
                rotation: rot_rz_ry_rx_deg(90.0, 0.0, 0.0),
                color: [1.0, 1.0, 1.0, col[3]],
                kind: Object3dKind::ZodiacRibbon {
                    kind: if let Consumable::Zodiac(z) = item.consumable {
                        Some(z)
                    } else {
                        None
                    },
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }

        // For-sale talismans: lower-right cabinet wall.
        for (i, item) in self.talisman_items.iter().enumerate() {
            if i >= layout.talisman_anchor_count {
                break;
            }
            let (ax, ay, awy) = layout.talisman_anchors_px[i];
            let mut col = consumable_color(item.consumable);
            if item.sold {
                col[3] = 0.30;
            }
            if let Consumable::Talisman(tk) = item.consumable {
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy],
                    extents: [
                        layout.talisman_wall_width * 1.4,
                        layout.talisman_wall_width * 2.0,
                        layout.talisman_wall_width * 0.35,
                    ],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: col,
                    kind: Object3dKind::Talisman { kind: tk },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }

        // Owned consumables.
        for (z_ord, owned) in shop.owned_zodiacs.iter().enumerate() {
            if let Consumable::Zodiac(z) = owned.consumable {
                let (ax, ay, awy) = layout.owned_ribbon_pos(z_ord);
                let w = layout.consumable_width;
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy],
                    extents: [w, layout.consumable_length, w * 0.15],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::ZodiacRibbon { kind: Some(z) },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }
        for (t_ord, owned) in shop.owned_talismans.iter().enumerate() {
            if let Consumable::Talisman(tk) = owned.consumable {
                let (ax, ay, awy) = layout.owned_talisman_pos(t_ord);
                let w = layout.consumable_width;
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy - layout.consumable_length * 0.4],
                    extents: [w * 1.4, w * 2.0, w * 0.35],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: consumable_color(owned.consumable),
                    kind: Object3dKind::Talisman { kind: tk },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.shelf.owned_talismans"),
                });
            }
        }

        if !consumable_objects.is_empty() {
            frame.object3d_batch(consumable_objects);
        }

        // ── Gold display: bars + coin strings inside the coin dish ─────
        let (bars, coins) = coin_display_layout(
            shop.display_gold,
            layout.coin_dish_center_px,
            layout.coin_dish_extents,
            self.age_secs,
        );
        if !bars.is_empty() {
            frame.object3d_batch(bars);
        }
        if !coins.is_empty() {
            frame.object3d_batch(coins);
        }

        // ── Shop lamp ─────────────────────────────────────────────────
        // An overhead pendant lamp hanging above the counter.
        // Mesh is in world-space Z-up convention: no corrective rotation.
        // `pos` is the apex (top of shade / cord attachment point).
        // The shade rim hangs below (at apex_z + SHADE_RIM_Z * scale_z).
        // The bulb sits at LAMP_BULB_LOCAL_Z (negative, below apex).
        let lamp_w = h * 0.22;
        let lamp_h = h * 0.30;
        // Hanging point: center-screen (world_x=0), far back (world_y ≈ h*0.35,
        // which is lamp_center_px.ny=0.15 → pixel_y=h*0.15),
        // at world_z = h*0.52 (lamp_lift_h_frac).
        let lp = layout.lamp_center_px;
        let lamp_hang_z = lp.2; // apex z — lamp hangs downward from here
        // Flicker: layered sines at incommensurate rates plus an occasional
        // brownout dip sell a failing bulb on a foggy mountain. Held in
        // `lamp_flicker` so the shade glow, point lights, and god-rays all
        // pulse in lockstep.
        let tf = self.age_secs;
        let flick_fast = (tf * 37.3).sin() * 0.04 + (tf * 61.7).sin() * 0.025;
        let flick_slow = (tf * 4.1).sin() * 0.06;
        let brownout = {
            let d = (tf * 0.73).sin() * (tf * 1.19).sin();
            (d - 0.55).max(0.0) * 0.35
        };
        let lamp_flicker = (1.0 + flick_fast + flick_slow - brownout).clamp(0.55, 1.12);
        frame.object3d(Object3d {
            pos: [lp.0, lp.1, lamp_hang_z],
            extents: [lamp_w, lamp_h, lamp_w],
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::ShopLamp { glow: lamp_flicker },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });

        // ── 3D bugs orbiting the bulb ──────────────────────────────────
        // Bugs orbit in world XY around the bulb. Three wobble layers:
        //   1. Bob — sinusoidal Z drift, each bug at a different frequency.
        //   2. Radius drift — orbit radius breathes in/out (moths lunge at light).
        //   3. Bank — body rolls into the turn; extra pitch when bobbing.
        {
            let t_now = self.age_secs;
            let bulb_wx = lp.0 - w * 0.5;
            let bulb_wy = h * 0.5 - lp.1;
            let bulb_wz = lamp_hang_z + LAMP_BULB_LOCAL_Z * lamp_h;
            let bug_body_len = h * 0.022;

            // Wing flap parameters. Real moths beat their wings ~20-30 Hz
            // (hawkmoths even faster); we pick 25 Hz for a species-accurate
            // feel. At 60 fps a single flap cycle is only ~2.4 frames, so
            // the live wing would strobe on its own — the swept-fan blur
            // surrogate mesh (`build_bug_wing_blur_mesh`) is what actually
            // sells this as motion blur rather than aliasing: the live wing
            // fades near mid-stroke and the pre-swept fan takes over, then
            // vice-versa at the turnarounds. Amplitude 1.1 rad (~63°) sweeps
            // from below horizontal up past vertical, matching the way moths
            // clap their wings above the body between strokes. Per-bug phase
            // offsets keep the swarm from flapping in unison.
            let flap_hz: f32 = 25.0;
            let flap_amp: f32 = 1.1;

            // Sample a bug's full transform at `t_back` seconds in the past.
            // Kept parametric (rather than inlined at t_back = 0) so callers
            // that want to predict a bug's pose at a nearby moment — e.g.
            // shadow prediction, debug overlays — can reuse the same math.
            // Returns `(pos, extents, body_rot, flap_rad)` where `flap_rad`
            // is the wing angle at that moment (rotating about body +X).
            let sample_bug = |i: usize, t_back: f32| -> ([f32; 3], [f32; 3], glam::Mat4, f32) {
                let (r_frac, z_frac, speed, size_frac) = BUG_PARAMS[i];
                let fi = i as f32;
                let t = t_now - t_back;
                let phase = self.bug_phases[i] - speed * t_back;

                let bob_freq = 2.3 + fi * 0.71;
                let drift_freq = 1.1 + fi * 0.43;
                let pitch_freq = 3.7 + fi * 0.57;

                let bob = (t * bob_freq + fi * 1.3).sin() * lamp_h * 0.15;
                let r_nom = lamp_w * r_frac;
                let r_drift = (t * drift_freq + fi * 2.1).sin() * r_nom * 0.20;
                let bug_wz = bulb_wz + lamp_h * z_frac + bob;

                let local_z = (bug_wz - lamp_hang_z) / lamp_h;
                let min_r_local = shade_exclusion_radius(local_z);
                // Clear the shade by the body radius plus the inside wing's
                // span. The body is oriented tangent to the orbit, so wings
                // extend radially — the inner wing is the one that could
                // clip the shade, and it reaches ~1.13 units in local Y
                // (see moth_wing_outline) scaled by `size_frac`.
                let wing_half_span = 1.13 * size_frac * bug_body_len;
                let min_r_world =
                    min_r_local * (lamp_w / SHADE_RIM_R) + bug_body_len * 0.6 + wing_half_span;
                let orbit_r = (r_nom + r_drift).max(min_r_world);

                let bug_wx = bulb_wx + orbit_r * phase.cos();
                let bug_wy = bulb_wy + orbit_r * phase.sin();
                let bug_px = bug_wx + w * 0.5;
                let bug_py = h * 0.5 - bug_wy;
                let bug_sz = bug_body_len * size_frac;

                let tx = -phase.sin();
                let ty = phase.cos();
                let bank = std::f32::consts::FRAC_PI_4 * 0.5 + (t * 1.9 + fi * 0.8).sin() * 0.30;
                let pitch = (t * pitch_freq + fi * 0.5).sin() * 0.25;
                let yaw = glam::Mat4::from_cols(
                    glam::Vec4::new(tx, ty, 0.0, 0.0),
                    glam::Vec4::new(-ty, tx, 0.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
                );
                let rot =
                    yaw * glam::Mat4::from_rotation_x(bank) * glam::Mat4::from_rotation_y(pitch);
                // Flap angle at time `t`. Sine wave in radians, offset per
                // bug index so the swarm's wingbeats are phase-staggered.
                let flap = flap_amp * (t * flap_hz * std::f32::consts::TAU + fi * 1.3).sin();
                (
                    [bug_px, bug_py, bug_wz],
                    [bug_sz, bug_sz, bug_sz],
                    rot,
                    flap,
                )
            };

            // Live bugs — the ghost-trail system is gone; each bug now emits
            // two swept-fan blur-surrogate draws (L/R) alongside its crisp
            // live wings. The live wing fades where the real wing would blur
            // (mid-stroke) and the blur fan fades where the real wing would
            // read crisply (turnarounds), producing a coherent moth that
            // looks like a 1/60 s exposure.
            for i in 0..BUG_COUNT {
                let (pos, extents, rot, flap_rad) = sample_bug(i, 0.0);
                // Angular-speed factor in [0, 1]: 0 at the flap turnarounds
                // (where sin() peaks, cos() is 0) and 1 at mid-stroke. This
                // is |d/dt sin(w t)| / max, which reduces to |cos(w t)|.
                let fi = i as f32;
                let speed_factor = (t_now * flap_hz * std::f32::consts::TAU + fi * 1.3)
                    .cos()
                    .abs();
                let live_wing_alpha = 1.0 - 0.7 * speed_factor;
                let blur_alpha = 0.6 * speed_factor;
                frame.object3d(Object3d {
                    pos,
                    extents,
                    rotation: rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Bug {
                        slot: i,
                        flap_rad,
                        live_wing_alpha,
                        blur_alpha,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
                // Shadow casters: body + two wings as separate Gaussian
                // occluders so the lamp's god-ray shafts show recognisable
                // moth silhouettes instead of round blobs. Wing centres
                // are mesh-local (±Y, ~0.40 out from body) rotated through
                // the bug's orientation matrix so banking/pitch rotate
                // the silhouette with the live mesh.
                //
                // Wing flap: the occluder offset along ±Y shrinks as the
                // wings sweep toward vertical (edge-on to the lamp) and
                // swells back to full when the wings lie flat. That's
                // what makes the shaft silhouettes "flap" in sync with
                // the visible mesh — without this, the shafts would show
                // a static two-wing shape while the mesh moves.
                //
                // `pos` is `[pixel_x, pixel_y, world_z]`; occluder storage
                // expects pixel-space XY with world-space Z. Converting a
                // world-space offset back to pixel coords is `(+dx, -dy)`
                // because pixel-Y points down while world-Y points up (see
                // `pixel_to_world` in `render/world_space.rs`).
                // Body is a slender near-cylindrical mesh; Y/Z radius is
                // 0.11 in mesh-local units (see `build_bug_body_mesh`).
                let body_r = extents[0] * 0.24;
                let flap_c = flap_rad.cos();
                let flap_s = flap_rad.sin();
                // Edge-on wings cast almost no shadow; use cos(flap) to
                // collapse the Gaussian radius toward zero at ±90°.
                // Centroid of the moth-wing outline sits around Y ≈ 0.55
                // in mesh-local units; the Gaussian radius scales with the
                // wing area projected onto the shaft plane (cos(flap)).
                let wing_r = extents[0] * (0.40 + 0.32 * flap_c.abs());
                let wing_offset_y = 0.55_f32 * flap_c;
                let wing_offset_z = 0.55_f32 * flap_s;
                // Body occluder — compact core at the bug's centre.
                frame
                    .bug_occluders
                    .push(crate::render::draw_cmd::BugOccluder {
                        center_px: (pos[0], pos[1]),
                        lift: pos[2],
                        radius: body_r,
                        strength: 28.0,
                    });
                // Wing occluders — rotated offsets from the body centre.
                // Left wing flaps to +Z, right to −Z (mirror across body).
                let wing_locals = [
                    glam::Vec3::new(0.0, wing_offset_y, wing_offset_z),
                    glam::Vec3::new(0.0, -wing_offset_y, wing_offset_z),
                ];
                for wl in wing_locals {
                    let rotated = rot.transform_vector3(wl * extents[0]);
                    let cx_px = pos[0] + rotated.x;
                    let cy_px = pos[1] - rotated.y;
                    let cz = pos[2] + rotated.z;
                    frame
                        .bug_occluders
                        .push(crate::render::draw_cmd::BugOccluder {
                            center_px: (cx_px, cy_px),
                            lift: cz,
                            radius: wing_r,
                            strength: 22.0,
                        });
                }
            }
        }

        // ── Back-wall smoke curtain ────────────────────────────────────
        // A column of wind impulses along the back of the scene seeds a
        // billowing curtain of density that the volumetric smoke pass
        // renders as a slow, rolling sheet behind the stall items.
        // Phase offsets per-emitter break the row up so it reads as a
        // drape, not a uniform wall.
        //
        // Arrange-mode: `positions.smoke_curtain` nudges the row's center
        // (`nx` horizontal, `ny` vertical) and lifts it (`lift_mm`). The
        // curtain has no mesh, so it's cycle-only via Tab (not clickable).
        // Live preview folds the staged arrange delta in here because
        // wind gusts don't go through `apply_arrange_override`.
        //
        // Magnitudes (density, radius, velocity, lift, emitter count) are
        // driven by `ctx.shop_smoke_tuning`, live-editable from the
        // "Shop Smoke..." debug overlay.
        let t = self.age_secs;
        let smoke = ctx.shop_smoke_tuning;
        let n_emitters = smoke.emitter_count.max(1) as usize;
        let curtain_p = match ctx.arrange_preview.as_ref() {
            Some(prev) => prev.applied_to(
                crate::ui::scene_layout::SHOP_HIERARCHY,
                "shop.props.smoke_curtain",
                self.positions.smoke_curtain,
            ),
            None => self.positions.smoke_curtain,
        };
        let curtain_cx = curtain_p.nx * w;
        let back_pixel_y = curtain_p.ny * h;
        let span = w * 0.88;
        let curtain_lift = h * smoke.lift_fraction + layout.mm(curtain_p.lift_mm);
        for i in 0..n_emitters {
            let f = if n_emitters <= 1 {
                0.0
            } else {
                i as f32 / (n_emitters as f32 - 1.0) - 0.5
            };
            let cx = curtain_cx + f * span;
            let phase = i as f32 * 1.37;
            // Three overlapping sines at different rates give the curtain a
            // rolling, non-repeating billow instead of a uniform sway.
            let sway = (t * 0.45 + phase).sin();
            let roll = (t * 0.72 + phase * 0.6).sin();
            let billow = (t * 0.31 + phase * 0.9).sin();
            // Forward pulse breathes the sheet toward/away from camera.
            let breathe = 0.5 + 0.5 * (t * 0.38 + phase * 0.45).sin();
            frame.wind_gusts.push(crate::render::draw_cmd::WindGust {
                center_px: (
                    cx + sway * w * 0.045 + billow * w * 0.02,
                    back_pixel_y + billow * h * 0.03,
                ),
                lift: curtain_lift + roll * h * 0.09 + billow * h * 0.05,
                velocity: [
                    sway * 14.0 + billow * 6.0,
                    -6.0 - roll * 5.0,
                    smoke.forward_velocity_base
                        + breathe * smoke.forward_velocity_breathe_amp
                        + roll * 4.0,
                ],
                radius: h * (smoke.radius_base + smoke.radius_billow_amp * billow),
                density: smoke.density_base
                    + smoke.density_roll_amp * roll
                    + smoke.density_billow_amp * billow,
            });
        }

        // ── Smoky atmosphere ───────────────────────────────────────────
        // The fluid smoke pass renders curling volumetric haze across
        // the screen, depth-aware so it pools around the cabinet and
        // dishes. This is what sells the "shop in a backroom under a
        // dim lamp" mood — without it the scene reads as 3D objects on
        // a flat black UI page.
        frame.fluid_smoke();

        // ── Lighting: cold fluorescent lamp + purple rim ───────────────
        // LAMP_BULB_LOCAL_Z is negative (below apex), scaled by lamp_h.
        let lamp_bulb_pos = [lp.0, lp.1, lamp_hang_z + LAMP_BULB_LOCAL_Z * lamp_h];
        let mut point_lights: Vec<PointLight> = vec![
            // Cold fluorescent key — slightly greenish-white, typical of tube lighting.
            PointLight {
                pos: [
                    layout.lamp_center_px.0,
                    layout.lamp_center_px.1,
                    layout.lamp_center_px.2,
                ],
                radius: h * 1.15,
                color: [0.86, 0.96, 0.98],
                intensity: 2.15 * lamp_flicker,
            },
            // Cold fluorescent fill at the bulb itself.
            PointLight {
                pos: lamp_bulb_pos,
                radius: h * 1.30,
                color: [0.82, 0.94, 1.00],
                intensity: 2.60 * lamp_flicker,
            },
            // Purple rim highlight — offset beside the bulb to catch edges.
            PointLight {
                pos: [
                    lamp_bulb_pos[0],
                    lamp_bulb_pos[1],
                    lamp_bulb_pos[2] - h * 0.04,
                ],
                radius: h * 0.70,
                color: [0.72, 0.38, 1.00],
                intensity: 1.80 * lamp_flicker,
            },
        ];

        // ── Hover spotlight: literal point light on the picked object ──
        // Uses the renderer's pick result so the spotlight is anchored to
        // the actual visible object the cursor is over.
        // Hover follows focus first (so controller / keyboard players see
        // the spotlight + tooltip on whatever they've navigated to), with
        // the cursor pick acting as a fallback for first-frame mouse
        // hovers before update() has had a chance to sync focus to the
        // cursor. Cursor mode `update()` writes `self.focus` from the
        // pick result already, so this expression collapses to "show the
        // focused element" in the steady state.
        let hover = self
            .focus
            .and_then(|f| f.to_hit())
            .or(ctx.picked_shop_object)
            .and_then(|hit| {
                live_shop_hit(
                    hit,
                    &self.items,
                    &self.zodiac_items,
                    &self.talisman_items,
                    &self.pack_items,
                    &shop,
                )
            });
        // Pre-compute action prop state (needed for hover_info and 3D chrome rendering).
        let reroll_affordable =
            self.mode == ShopMode::Standard && shop.gold >= self.reroll_cost as i32;
        // Pre-compute hover item info for both the 3D plaque and the 2D description overlay.
        let n_for_sale_relics_hud = self.items.len().min(layout.niche_count);
        let hover_info: Option<(String, String, String, [f32; 4])> = hover.map(|hit| {
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            match hit {
                ShopHit::Relic(i) if i < n_for_sale_relics_hud => {
                    let item = &self.items[i];
                    let can_afford =
                        shop.gold >= item.price as i32 && !shop.relics_full && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if shop.relics_full {
                            "Relics full".to_string()
                        } else {
                            format!("${} (have {}g)", item.price, shop.display_gold)
                        }
                    } else {
                        item.buy_label()
                    };
                    let col = if item.sold {
                        color::SLATE
                    } else if can_afford {
                        color::GOLD
                    } else {
                        color::RUBY
                    };
                    (
                        item.name.to_string(),
                        item.description.to_string(),
                        cta,
                        col,
                    )
                }
                ShopHit::Relic(i) => {
                    let oi = i - n_for_sale_relics_hud;
                    if oi < shop.owned_relics.len() {
                        let rid = shop.owned_relics[oi];
                        let defs = all_relic_defs();
                        let def = defs.iter().find(|d| d.id == rid);
                        let name = def
                            .map(|d| d.name.to_string())
                            .unwrap_or_else(|| "Relic".into());
                        let desc = relic_description_live(
                            rid,
                            &shop.relic_counters,
                            shop.total_score_earned,
                        );
                        let sell = relic_sell_price_live(rid, &shop.relic_counters);
                        (name, desc, format!("Sell {}g", sell), color::CHAMPAGNE)
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
                    let item = &self.zodiac_items[i];
                    let price = item.price(&ctx.run.mode);
                    let can_afford = shop.gold >= price as i32 && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        format!("${} (have {}g)", price, shop.display_gold)
                    } else if price == 0 {
                        "FREE".to_string()
                    } else {
                        format!("Buy {}g", price)
                    };
                    let col = if item.sold {
                        color::SLATE
                    } else if can_afford {
                        color::GOLD
                    } else {
                        color::RUBY
                    };
                    (item.name(), item.description(), cta, col)
                }
                ShopHit::Ribbon(i) => {
                    let oi = i - n_for_sale_zodiacs;
                    if let Some(c) = shop.owned_zodiacs.get(oi).map(|item| item.consumable) {
                        let item = ConsumableShopItem {
                            consumable: c,
                            sold: false,
                        };
                        (
                            item.name(),
                            item.description(),
                            "Use".to_string(),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Talisman(i) if i < n_for_sale_talismans => {
                    let item = &self.talisman_items[i];
                    let price = item.price(&ctx.run.mode);
                    let can_afford =
                        shop.gold >= price as i32 && !shop.consumables_full && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if shop.consumables_full {
                            "Inventory full".to_string()
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
                        }
                    } else if price == 0 {
                        "FREE".to_string()
                    } else {
                        format!("Buy {}g", price)
                    };
                    let col = if item.sold {
                        color::SLATE
                    } else if can_afford {
                        color::GOLD
                    } else {
                        color::RUBY
                    };
                    (item.name(), item.description(), cta, col)
                }
                ShopHit::Talisman(i) => {
                    let oi = i - n_for_sale_talismans;
                    if let Some(c) = shop.owned_talismans.get(oi).map(|item| item.consumable) {
                        let item = ConsumableShopItem {
                            consumable: c,
                            sold: false,
                        };
                        (
                            item.name(),
                            item.description(),
                            format!("Sell {}g", consumable_sell_price_for_mode(c, &ctx.run.mode)),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Dish(id) if id == PICK_COIN_DISH => (
                    "Gold".to_string(),
                    "Your current treasure".to_string(),
                    format!("{}g", shop.gold),
                    color::GOLD,
                ),
                ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK => (
                    "Yaku Journal".to_string(),
                    "Levels, plays, and how to build every yaku".to_string(),
                    "Open".to_string(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if id == PICK_LEAVE_PROP => (
                    if self.mode == ShopMode::Tutorial {
                        "Face Boss"
                    } else {
                        "Continue On"
                    }
                    .to_string(),
                    "Continue to the next round".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if id == PICK_REROLL_PROP => (
                    "Restock".to_string(),
                    format!("Refresh shop for {}g", self.reroll_cost),
                    if reroll_affordable {
                        format!("{}g", self.reroll_cost)
                    } else {
                        format!("${} (have {}g)", self.reroll_cost, shop.display_gold)
                    },
                    if reroll_affordable {
                        color::GOLD
                    } else {
                        color::RUBY
                    },
                ),
                ShopHit::Dish(id) if id == PICK_SELL_TRAY => (
                    "Sell".to_string(),
                    "Focus an owned relic or consumable, then click here to sell it".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if is_tile_pack_pick(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    if let Some(pack) = self.pack_items.get(idx) {
                        let price = pack.kind.shop_price();
                        let can_afford = shop.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
                        };
                        (
                            pack.kind.name().to_string(),
                            pack.kind.description().to_string(),
                            cta,
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Dish(_) => (
                    "Relic dish".to_string(),
                    "Hover an owned relic to sell it".to_string(),
                    String::new(),
                    color::SLATE,
                ),
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    if let Some(pack) = self.pack_items.get(idx) {
                        let price = pack.kind.shop_price();
                        let can_afford = shop.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
                        };
                        let col = if pack.sold {
                            color::SLATE
                        } else if can_afford {
                            color::CHAMPAGNE
                        } else {
                            color::SLATE
                        };
                        (
                            pack.kind.name().to_string(),
                            pack.kind.description().to_string(),
                            cta,
                            col,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
            }
        });

        if let Some(hit) = hover {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            // Helper: get the (px, py, wy) anchor of a hit consumable for
            // spotlight placement. Walks the same partition the renderer
            // uses (for-sale-of-kind, then owned-of-kind) to find which
            // wall slot or fan position to light up.
            let zodiac_anchor = |hit_idx: usize| -> Option<(f32, f32, f32)> {
                let n_for_sale = self.zodiac_items.len();
                if hit_idx < n_for_sale {
                    if hit_idx < layout.ribbon_count {
                        return Some(layout.ribbon_anchors_px[hit_idx]);
                    }
                    return None;
                }
                let owned_target = hit_idx - n_for_sale;
                if owned_target < shop.owned_zodiacs.len() {
                    return Some(layout.owned_ribbon_pos(owned_target));
                }
                None
            };
            let talisman_anchor = |hit_idx: usize| -> Option<(f32, f32, f32)> {
                let n_for_sale = self.talisman_items.len();
                if hit_idx < n_for_sale {
                    if hit_idx < layout.talisman_anchor_count {
                        return Some(layout.talisman_anchors_px[hit_idx]);
                    }
                    return None;
                }
                let owned_target = hit_idx - n_for_sale;
                if owned_target < shop.owned_talismans.len() {
                    return Some(layout.owned_talisman_pos(owned_target));
                }
                None
            };
            match hit {
                ShopHit::Relic(i) => {
                    let (px, py, wy) = if i < n_for_sale_relics {
                        layout.niche_centers_px[i]
                    } else {
                        let oi = i - n_for_sale_relics;
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    };
                    point_lights.push(PointLight {
                        pos: [px, py - 30.0, wy + 60.0],
                        radius: 180.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
                    });
                }
                ShopHit::Ribbon(i) => {
                    if let Some((px, py, wy)) = zodiac_anchor(i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 40.0, wy - layout.ribbon_length * 0.4],
                            radius: 200.0,
                            color: [1.00, 0.92, 0.74],
                            intensity: 3.00,
                        });
                    }
                }
                ShopHit::Talisman(i) => {
                    if let Some((px, py, wy)) = talisman_anchor(i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 30.0, wy + 40.0],
                            radius: 180.0,
                            color: [0.78, 1.00, 0.82],
                            intensity: 3.20,
                        });
                    }
                }
                ShopHit::Dish(id) => {
                    let center = if id == PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else {
                        layout.coin_dish_center_px
                    };
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 20.0, 80.0],
                        radius: 220.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 2.50,
                    });
                }
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    let center = layout
                        .pack_centers_px
                        .get(idx)
                        .copied()
                        .unwrap_or(layout.pack_centers_px[0]);
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 30.0, center.2 + 60.0],
                        radius: 180.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
                    });
                }
            }
        }
        frame.point_lights = point_lights;

        // ── Hover item 3D anchor for plaque placement ───────────────────
        // Resolves the world-space top-face anchor of the currently hovered
        // item's AABB so the title plaque floats above it. Returned as
        // (pixel_x, pixel_y, world_z).
        let hover_item_pos: Option<(f32, f32, f32)> = hover.map(|hit| {
            if let Some(&[px, py, wz]) = self.hover_anchor_overrides.get(&hit) {
                return (px, py, wz);
            }
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            match hit {
                ShopHit::Relic(i) => {
                    if i < n_for_sale_relics {
                        let (px, py, wy) = layout.niche_centers_px[i];
                        let niche_base = layout.counter_extents[0] * 0.055;
                        let half = relic_half_extents(self.items[i].relic, niche_base);
                        (px, py, wy + half[2] * 2.0)
                    } else {
                        let oi = i - n_for_sale_relics;
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    }
                }
                ShopHit::Ribbon(i) => {
                    let n_for_sale = self.zodiac_items.len();
                    if i < n_for_sale {
                        if i < layout.ribbon_count {
                            layout.ribbon_anchors_px[i]
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    } else {
                        let oi = i - n_for_sale;
                        layout.owned_ribbon_pos(oi)
                    }
                }
                ShopHit::Talisman(i) => {
                    let n_for_sale = self.talisman_items.len();
                    if i < n_for_sale {
                        if i < layout.talisman_anchor_count {
                            let (ax, ay, awy) = layout.talisman_anchors_px[i];
                            (ax, ay, awy + layout.talisman_wall_width)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    } else {
                        let oi = i - n_for_sale;
                        layout.owned_talisman_pos(oi)
                    }
                }
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    let center = layout
                        .pack_centers_px
                        .get(idx)
                        .copied()
                        .unwrap_or(layout.pack_centers_px[0]);
                    (center.0, center.1, center.2 + layout.pack_extents[2])
                }
                ShopHit::Dish(id) => {
                    if id == PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else if id == PICK_COIN_DISH {
                        layout.coin_dish_center_px
                    } else if id == PICK_REROLL_PROP {
                        (
                            self.positions.reroll_prop.nx * w,
                            layout.counter_world_y + h * 0.5,
                            layout.mm(self.positions.reroll_prop.lift_mm),
                        )
                    } else if id == PICK_LEAVE_PROP {
                        (
                            self.positions.leave_prop.nx * w,
                            layout.counter_world_y + h * 0.5,
                            layout.mm(self.positions.leave_prop.lift_mm),
                        )
                    } else if id == PICK_SELL_TRAY {
                        let vis_px_min = w * 0.25;
                        let vis_w2 = w * 0.5;
                        (
                            vis_px_min + self.positions.sell_tray.nx * vis_w2,
                            layout.relic_dish_center_px.1,
                            layout.mm(self.positions.sell_tray.lift_mm),
                        )
                    } else {
                        (w * 0.5, h * 0.5, 0.0)
                    }
                }
            }
        });

        // ── 3D shop chrome: Ofuda sign, info plaque, action props, sell tray ──
        let cam_rot = camera_facing_rotation(layout.camera.eye, layout.camera.target);

        // Path sign: Ofuda scroll. Arrange-mode placement contributes
        // additive deltas (position, lift, rotation) on top of the baked-in
        // -82° pitch and the counter-relative anchor.
        let ofuda_p = &self.positions.ofuda;
        frame.object3d(Object3d {
            pos: [
                w * 0.23 + ofuda_p.nx * w,
                layout.counter_world_y - h * 0.057 + h * 0.5 + ofuda_p.ny * h,
                layout.mm(147.66) + layout.mm(ofuda_p.lift_mm),
            ],
            extents: [w * 0.2, h * 0.12, layout.mm(3.0)],
            // Placement rx/ry/rz_deg are applied centrally by the renderer
            // via `committed_arrange_rotations`; keep only the baseline tilt
            // and the camera-facing yaw here.
            rotation: glam::Mat4::from_rotation_x((-82.0_f32).to_radians()) * cam_rot,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::Ofuda,
                material: crate::render::primitive::MaterialSpec::plain().with_decal(
                    crate::render::primitive::DecalSpec {
                        text: format!("{}\n{}", plaque_top_text, plaque_bot_text),
                        palette: crate::render::primitive::DecalPalette::ParchmentInk,
                        layout: crate::render::primitive::DecalLayout::TitleRule {
                            target_short_edge: crate::render::decal::OFUDA_DECAL_LONG_EDGE,
                        },
                    },
                ),
                pick_id: None,
                shadow_caster: false,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.props.ofuda"),
        });

        // Gold counter plaque — permanent slab on the counter near the coin
        // dish, showing the player's current gold at a glance.
        frame.object3d(Object3d {
            pos: [
                self.positions.coin_dish.nx * w,
                layout.counter_world_y + h * 0.5 - 60.0,
                layout.mm(self.positions.coin_dish.lift_mm) + layout.mm(10.0),
            ],
            extents: [w * 0.09, layout.mm(22.0), h * 0.045],
            rotation: cam_rot,
            color: [0.88, 0.78, 0.42, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::BeveledSlab,
                material: crate::render::primitive::MaterialSpec::lacquered_wood_flat().with_decal(
                    crate::render::primitive::plaque_decal(format!("Gold\n{}g", shop.display_gold)),
                ),
                pick_id: None,
                shadow_caster: false,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0xAC10,
            arrange_name: Some("shop.shelf.coin_dish"),
        });

        // Info plaque: title+CTA above the hovered item.
        //
        // Player-owned items live on the bottom-shelf trays (y≈0.84h), so the
        // for-sale layout — title above, description below — pushes the
        // description off the bottom of the screen. For owned items we draw a
        // single combined plaque (title / sell price / description) anchored
        // at its own arrange-mode placement so it can be positioned
        // independently from the for-sale hover plaques.
        let hover_is_owned = match hover {
            Some(ShopHit::Relic(i)) => i >= n_for_sale_relics_hud,
            Some(ShopHit::Ribbon(i)) => i >= self.zodiac_items.len(),
            Some(ShopHit::Talisman(i)) => i >= self.talisman_items.len(),
            _ => false,
        };
        if let Some((ref title, ref desc, ref cta, _)) = hover_info
            && !title.is_empty()
            && let Some((tpx, tpy, twz)) = hover_item_pos
        {
            let plaque_rot = glam::Mat4::from_rotation_x((-80.0_f32).to_radians()) * cam_rot;
            let plaque_z = layout.mm(4.0);

            // Clamp a plaque's pixel-X so its full width stays inside
            // the camera frustum at the plaque's world depth. Without
            // this, plaques anchored to edge items (far-left relics or
            // far-right ribbons) slide past the screen edge.
            let clamp_plaque_px = |center_px: f32, plaque_w: f32, py: f32, wz: f32| -> f32 {
                let world_y = h * 0.5 - py;
                let (fw_min, fw_max) = layout.camera.frustum_x_range_at(w, h, world_y, wz);
                let px_min = (fw_min + w * 0.5).max(0.0);
                let px_max = (fw_max + w * 0.5).min(w);
                let margin = w * 0.01;
                let lo = px_min + plaque_w * 0.5 + margin;
                let hi = px_max - plaque_w * 0.5 - margin;
                if hi <= lo {
                    (px_min + px_max) * 0.5
                } else {
                    center_px.clamp(lo, hi)
                }
            };

            if hover_is_owned {
                // Combined owned-item plaque: title / sell price /
                // description, stacked on one sign. Width sized like
                // the description plaque so long relic descriptions
                // don't shrink against the font clamp; height sized
                // to wrapped content.
                let owned_p = &self.positions.hover_owned_plaque;
                let plaque_w = w * 0.38;
                let text = if desc.is_empty() {
                    format!("{}\n{}", title, cta)
                } else {
                    format!("{}\n{}\n{}", title, cta, desc)
                };
                let font_px = (h * 0.022).max(14.0);
                let pad_frac = 0.1;
                let inner_w = plaque_w * (1.0 - 2.0 * pad_frac);
                let (line_count, line_h) =
                    measure_plaque_wrap(load_ui_font().as_ref(), &text, inner_w, font_px);
                let content_h = line_count as f32 * line_h;
                let plaque_h = (content_h / (1.0 - 2.0 * pad_frac)).max(h * 0.10);

                let py = tpy + owned_p.ny * h;
                let wz = (twz + h * 0.05 + layout.mm(owned_p.lift_mm)).max(0.0);
                let px = clamp_plaque_px(tpx + owned_p.nx * w, plaque_w, py, wz);
                frame.object3d(Object3d {
                    pos: [px, py, wz],
                    extents: [plaque_w, plaque_h, plaque_z],
                    rotation: plaque_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::BeveledSlab,
                        material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                            .with_decal(crate::render::primitive::plaque_decal(text)),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0xAC0E,
                    arrange_name: Some("shop.hover.owned_plaque"),
                });
            } else {
                let plaque_w = w * 0.22;
                let plaque_h = h * 0.12;

                // Title plaque: anchored to the top face of the
                // item's AABB, floated up in screen-space and pulled
                // forward toward the camera. Arrange-mode placement
                // contributes additive deltas.
                let title_p = &self.positions.hover_title_plaque;
                let title_py = tpy - h * 0.28 + title_p.ny * h;
                let title_wz = twz + h * 0.14 + layout.mm(title_p.lift_mm);
                let title_px = clamp_plaque_px(tpx + title_p.nx * w, plaque_w, title_py, title_wz);
                frame.object3d(Object3d {
                    pos: [title_px, title_py, title_wz],
                    extents: [plaque_w, plaque_h, plaque_z],
                    rotation: plaque_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::BeveledSlab,
                        material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                            .with_decal(crate::render::primitive::plaque_decal(format!(
                                "{}\n{}",
                                title, cta
                            ))),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0xAC00,
                    arrange_name: Some("shop.hover.title_plaque"),
                });

                // Description plaque: anchored below the item so the
                // player can read what the focused relic/ribbon/
                // talisman actually does.
                if !desc.is_empty() {
                    let desc_p = &self.positions.hover_desc_plaque;
                    // Wider + shorter than the title plaque.
                    let desc_w = w * 0.38;
                    let font_px = (h * 0.022).max(14.0);
                    let pad_frac = 0.1;
                    let inner_w = desc_w * (1.0 - 2.0 * pad_frac);
                    let (line_count, line_h) =
                        measure_plaque_wrap(load_ui_font().as_ref(), desc, inner_w, font_px);
                    let content_h = line_count as f32 * line_h;
                    let desc_h = (content_h / (1.0 - 2.0 * pad_frac)).max(h * 0.08);
                    let desc_py = tpy + h * 0.10 + desc_p.ny * h;
                    let desc_wz = (twz - h * 0.10 + layout.mm(desc_p.lift_mm)).max(0.0);
                    let desc_px = clamp_plaque_px(tpx + desc_p.nx * w, desc_w, desc_py, desc_wz);
                    frame.object3d(Object3d {
                        pos: [desc_px, desc_py, desc_wz],
                        extents: [desc_w, desc_h, plaque_z],
                        rotation: plaque_rot,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Object3dKind::Primitive {
                            shape: crate::render::primitive::MeshId::BeveledSlab,
                            material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                                .with_decal(crate::render::primitive::plaque_decal(desc.clone())),
                            pick_id: None,
                            shadow_caster: false,
                            silhouette: false,
                        },
                        hover_target: 0.0,
                        anim_id: 0xAC0D,
                        arrange_name: Some("shop.hover.desc_plaque"),
                    });
                }
            }
        }

        // Reroll prop — left end of counter.
        let reroll_label = if self.mode == ShopMode::Tutorial {
            "Curated Stock".to_string()
        } else {
            format!("Restock {}g", self.reroll_cost)
        };
        let reroll_color = if reroll_affordable {
            [0.85, 0.78, 0.55, 1.0]
        } else {
            [0.45, 0.42, 0.35, 1.0]
        };
        let hover_is_reroll = matches!(hover, Some(ShopHit::Dish(id)) if id == PICK_REROLL_PROP);
        {
            use crate::render::primitive::{
                DecalLayout, DecalPalette, DecalSpec, MaterialSpec, MeshId,
            };
            let disabled = !reroll_affordable;
            let alpha = if disabled { 0.45 } else { reroll_color[3] };
            let color = [reroll_color[0], reroll_color[1], reroll_color[2], alpha];
            frame.object3d(Object3d {
                pos: [
                    self.positions.reroll_prop.nx * w,
                    layout.counter_world_y + h * 0.5,
                    layout.mm(self.positions.reroll_prop.lift_mm),
                ],
                extents: [w * 0.09, layout.mm(35.0), h * 0.065],
                rotation: cam_rot,
                color,
                kind: Object3dKind::Primitive {
                    shape: MeshId::ShopActionProp,
                    material: MaterialSpec {
                        kind: crate::render::lit_mesh::MaterialKind::Plain,
                        specular_strength: 0.4,
                        specular_power: 32.0,
                        decal: Some(DecalSpec {
                            text: reroll_label,
                            palette: DecalPalette::GoldGilded,
                            layout: DecalLayout::Fixed {
                                width: 512,
                                height: 192,
                            },
                        }),
                    },
                    pick_id: Some(PICK_REROLL_PROP),
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: if hover_is_reroll { 1.0 } else { 0.0 },
                anim_id: 0xAC01,
                arrange_name: Some("shop.props.reroll_prop"),
            });
        }

        // Leave prop — right end of counter.
        let leave_label = if self.mode == ShopMode::Tutorial {
            "Face Boss"
        } else {
            "Continue On"
        };
        let hover_is_leave = matches!(hover, Some(ShopHit::Dish(id)) if id == PICK_LEAVE_PROP);
        {
            use crate::render::primitive::{
                DecalLayout, DecalPalette, DecalSpec, MaterialSpec, MeshId,
            };
            frame.object3d(Object3d {
                pos: [
                    self.positions.leave_prop.nx * w,
                    layout.counter_world_y + h * 0.5,
                    layout.mm(self.positions.leave_prop.lift_mm),
                ],
                extents: [w * 0.09, layout.mm(35.0), h * 0.065],
                rotation: cam_rot,
                color: [0.92, 0.88, 0.72, 1.0],
                kind: Object3dKind::Primitive {
                    shape: MeshId::ShopActionProp,
                    material: MaterialSpec {
                        kind: crate::render::lit_mesh::MaterialKind::Plain,
                        specular_strength: 0.4,
                        specular_power: 32.0,
                        decal: Some(DecalSpec {
                            text: leave_label.to_string(),
                            palette: DecalPalette::GoldGilded,
                            layout: DecalLayout::Fixed {
                                width: 512,
                                height: 192,
                            },
                        }),
                    },
                    pick_id: Some(PICK_LEAVE_PROP),
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: if hover_is_leave { 1.0 } else { 0.0 },
                anim_id: 0xAC02,
                arrange_name: Some("shop.props.leave_prop"),
            });
        }

        // Yaku Journal — wood action tablet styled like gameplay's
        // action-bar journal button. Replaces the 3D book prop so the
        // journal affordance reads the same across scenes. Click
        // routes through `ShopHit::Dish(PICK_JOURNAL_BOOK)` via the
        // WoodTablet dispatch's `pick_id` hook.
        frame.object3d(Object3d {
            pos: [journal_cx, journal_cy, journal_cz],
            extents: [w * 0.06, layout.mm(16.0), h * 0.11],
            rotation: cam_rot,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::WoodTablet {
                label: "Journal".to_string(),
                pick_id: Some(PICK_JOURNAL_BOOK),
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.props.journal"),
        });

        // Sell tray — bottom shelf row, accessible from all item types.
        // Highlight when an item is being dragged toward it or when a sellable
        // item is focused (keyboard/controller mode).
        let has_sellable_focus = focused_sell_action(
            self.focus,
            self.items.len(),
            &self.zodiac_items,
            &self.talisman_items,
            &shop,
        )
        .is_some();
        let has_active_drag = self.held_item_drag.is_some() || self.mouse_drag.is_some();
        let sell_tray_color = if has_sellable_focus || has_active_drag {
            [0.70, 0.90, 0.60, 1.0]
        } else {
            [0.45, 0.55, 0.45, 0.7]
        };
        let sell_tray_px_x = {
            let vis_px_min = w * 0.25;
            let vis_w = w * 0.5;
            vis_px_min + self.positions.sell_tray.nx * vis_w
        };
        frame.object3d(Object3d {
            pos: [
                sell_tray_px_x,
                layout.relic_dish_center_px.1,
                layout.mm(self.positions.sell_tray.lift_mm),
            ],
            extents: [w * 0.09, layout.mm(4.0), h * 0.065],
            rotation: glam::Mat4::IDENTITY,
            color: sell_tray_color,
            kind: Object3dKind::SellTray {
                pick_id: Some(PICK_SELL_TRAY),
            },
            hover_target: 1.0,
            anim_id: 0xAC03,
            arrange_name: Some("shop.props.sell_tray"),
        });

        // ── 2D HUD: tooltip + chrome buttons ───────────────────────────
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();
        let _ = ctx.layout.score_panel;

        // ── Tutorial shop banner ────────────────────────────────────────
        // When the tutorial is active and the lesson is shop-enabled, show
        // a hint banner at the top of the screen guiding the player through
        // the shop UI — mirroring the gameplay scene's tutorial overlay
        // style.
        if self.mode == ShopMode::Tutorial {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            let has_bought = !shop.owned_relics.is_empty()
                || !shop.owned_zodiacs.is_empty()
                || !shop.owned_talismans.is_empty()
                || shop.full_hand_level > 1
                || (!self.pack_items.is_empty() && self.pack_items.iter().any(|p| p.sold));
            let (flavor, hint) = if has_bought {
                (
                    "Your loadout is ready.",
                    "Try selling an owned item if you want to see the refund flow, or use LB / RB on owned relics to reorder them before you face The Iconoclast.",
                )
            } else if self.items.is_empty() {
                ("The kiosk is bare\u{2026}", "Press Leave to move on.")
            } else if let Some(hit) = hover {
                match hit {
                    ShopHit::Relic(i) if i < n_for_sale_relics => (
                        "Relics are permanent run upgrades.",
                        "The left stall sells passive relics. Read the tooltip, check the gold cost, and buy the one that best helps your scoring plan.",
                    ),
                    ShopHit::Relic(_) => (
                        "Owned relics live in the lower-left tray.",
                        "Hover a relic in the tray to review its effect. Use the Sell button or press LB / [ to cash it out if you want to pivot your build.",
                    ),
                    ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => (
                        "Ribbons level up a yaku.",
                        "Buying a ribbon boosts one scoring pattern for the rest of the run. They are great when you already know which yaku you want to chase.",
                    ),
                    ShopHit::Talisman(i) if i < n_for_sale_talismans => (
                        "Talismans are consumable upgrades.",
                        "Talismans go into your consumable tray and modify tiles or scoring. They are flexible pickups when you do not want to commit to a relic.",
                    ),
                    ShopHit::Ribbon(_) => (
                        "Owned ribbons can be used here.",
                        "Hover an owned ribbon in the bottom ribbon tray and click Use to apply its yaku level-up before the next blind.",
                    ),
                    ShopHit::Talisman(_) => (
                        "Owned talismans can be sold back.",
                        "Hover an owned talisman in the bottom talisman tray to inspect it or sell it for gold if you need room.",
                    ),
                    ShopHit::Dish(id) if is_tile_pack_pick(id) => (
                        "Tile packs change the wall.",
                        "Packs add new tiles to future draws. They are optional, but can reshape the kinds of melds your run wants to make.",
                    ),
                    ShopHit::TilePack(_) => (
                        "Tile packs change the wall.",
                        "Packs add new tiles to future draws. They are optional, but can reshape the kinds of melds your run wants to make.",
                    ),
                    _ => (
                        "Take a look around the Shop.",
                        "Hover any item to inspect it. The tooltip tells you what it does and whether you can buy, use, or sell it.",
                    ),
                }
            } else {
                (
                    "Welcome to the Shop!",
                    "Four stalls: relics, packs, talismans, ribbons. Hover anything to inspect it, then buy what helps before pressing Leave.",
                )
            };

            let alpha = 1.0_f32;
            let flavor_px = typography::size(typography::BODY, h, ui_scale).max(15.0);
            let hint_px = typography::size(typography::BODY, h, ui_scale).max(15.0);
            let pad = (16.0 * ui_scale).max(10.0);

            // Right-side vertical panel — sits below the zodiac area so
            // it never overlaps relic tooltips in the upper-left.
            let banner_w = (w * 0.30).clamp(320.0, 460.0);
            let banner_x = w - banner_w - w * 0.02;
            let banner_y = h * 0.40;
            let text_w = banner_w - pad * 2.0;

            // Pre-wrap both text blocks to compute dynamic height.
            let flavor_line_h = flavor_px * 1.4;
            let flavor_lines = widget::wrap_text(flavor, text_w, flavor_px);
            let flavor_h = flavor_lines.len().max(1) as f32 * flavor_line_h;
            let hint_line_h = hint_px * 1.4;
            let hint_lines = widget::wrap_text(hint, text_w, hint_px);
            let hint_h = hint_lines.len().max(1) as f32 * hint_line_h;
            let banner_h = (pad + flavor_h + pad * 0.5 + hint_h + pad)
                .min(h - banner_y - (92.0 * ui_scale).max(72.0));

            // Gold border.
            let border = 2.0;
            quads.push(GpuInstance {
                rect: [
                    banner_x - border,
                    banner_y - border,
                    banner_w + border * 2.0,
                    banner_h + border * 2.0,
                ],
                color: [
                    color::BRASS[0],
                    color::BRASS[1],
                    color::BRASS[2],
                    0.4 * alpha,
                ],
            });
            // Dark panel.
            quads.push(GpuInstance {
                rect: [banner_x, banner_y, banner_w, banner_h],
                color: [
                    color::MIDNIGHT[0],
                    color::MIDNIGHT[1],
                    color::MIDNIGHT[2],
                    0.88 * alpha,
                ],
            });
            // Flavor text (gold, left-aligned for narrow panel).
            let flavor_y = banner_y + pad;
            widget::push_text_block(
                &mut texts,
                [banner_x + pad, flavor_y, text_w, flavor_h],
                flavor,
                TextStyle {
                    tier: typography::BODY,
                    color: [color::GOLD[0], color::GOLD[1], color::GOLD[2], 0.8 * alpha],
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
            // Hint text (champagne, left-aligned).
            let hint_y = flavor_y + flavor_h + pad * 0.5;
            widget::push_text_block(
                &mut texts,
                [banner_x + pad, hint_y, text_w, hint_h],
                hint,
                TextStyle {
                    tier: typography::BODY,
                    color: [
                        color::CHAMPAGNE[0],
                        color::CHAMPAGNE[1],
                        color::CHAMPAGNE[2],
                        alpha,
                    ],
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
        }

        let scale = metrics::scene_scale(w, h, ui_scale);

        // ── Focus rect graph + brass focus ring ────────────────────────
        //
        // Build a single list of `(ShopFocus, screen_rect)` covering
        // every navigable shop element this frame, then stash it for
        // `update()` to consume next frame. Same one-frame-stale pattern
        // the gameplay scene uses.
        //
        // Source rects:
        //   - Relics: `projected_relic_rects` (in-cabinet then in-dish order)
        //   - Ribbons: `projected_ribbon_rects`
        //   - Talismans: `projected_talisman_rects`
        //   - Dishes (incl. Leave/Reroll/SellTray props): `aux_dish_rects` paired with pick id
        let mut focus_rect_graph: Vec<(ShopFocus, [f32; 4])> = Vec::new();
        for (i, r) in ctx.proj.relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Relic(i), *r));
            }
        }
        for (i, r) in ctx.proj.ribbon_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Ribbon(i), *r));
            }
        }
        for (i, r) in ctx.proj.talisman_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Talisman(i), *r));
            }
        }
        for (pid, r) in ctx.proj.aux_dish_rects.iter() {
            if r[2] > 1.0
                && r[3] > 1.0
                && r[0].is_finite()
                && r[1].is_finite()
                && let Some(id) = pid
            {
                if is_tile_pack_pick(*id) {
                    focus_rect_graph.push((ShopFocus::Pack(*id), *r));
                } else if *id == PICK_LEAVE_PROP {
                    focus_rect_graph.push((ShopFocus::NextRound, *r));
                } else if *id == PICK_REROLL_PROP {
                    focus_rect_graph.push((ShopFocus::Reroll, *r));
                } else if *id == PICK_SELL_TRAY {
                    focus_rect_graph.push((ShopFocus::SellTray, *r));
                } else if *id != PICK_RELIC_DISH {
                    focus_rect_graph.push((ShopFocus::Dish(*id), *r));
                }
            }
        }

        // Push the brass focus ring on top of the 2D HUD layer so it
        // sits above the cabinet wood and dishes. Skipped during pause
        // because the overlay's own buttons take focus.
        if !self.pause_menu.paused
            && let Some(target) = self.focus
        {
            let rect_lookup = focus_rect_graph
                .iter()
                .find_map(|(t, r)| (*t == target).then_some(*r));
            if let Some(rect) = rect_lookup {
                push_focus_ring(rect, scale, w, h, &mut quads);
            }
        }

        for (i, item) in self.items.iter().enumerate() {
            if item.sold || item.price != 0 {
                continue;
            }
            if let Some(rect) = ctx.proj.relic_rects.get(i).copied() {
                push_free_badge(&mut quads, &mut texts, rect, h, ui_scale);
            }
        }

        // Stash for next frame's update().
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

        // The `?` glossary badge has been removed — the glossary is
        // reachable from the pause menu's "Glossary" entry. The keyboard
        // `Help` action shortcut still works for power users.
        // ── Catch-all 3D-hit dispatcher ───────────────────────────────
        // Full-screen button registered LAST so it only wins if no other
        // (smaller) button matched the cursor first.
        buttons.push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));

        // Pause overlay. While paused, drop all shop buttons (next-round,
        // help badge, full-screen 3D catch-all, etc.) so the pause menu's
        // own buttons are the only clickable surfaces — otherwise the
        // SHOP_3D_HIT_ID full-screen catch-all above would intercept every
        // click before the pause buttons even get tested.
        if self.pause_menu.paused {
            buttons.clear();
        }
        self.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: w,
                window_h: h,
                ui_scale,
            },
            scale,
            &mut quads,
            &mut texts,
            &mut buttons,
        );
        // Fullscreen click-blocker behind the pause menu's own buttons so
        // missed clicks become no-ops instead of falling through.
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Push 2D layers onto the frame after all 3D content.
        frame.quads(quads);
        frame.texts(texts);

        // ── Tile-pack opening celebration overlay ─────────────────────
        if let Some(ref celeb) = self.pack_celebration {
            let n = celeb.tiles.len();
            // Semi-transparent dimmer over the whole shop.
            frame.quad(GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.0, 0.0, 0.0, 0.72],
            });

            // Title: pack name — above the content area.
            let title_font = (h * 0.045).max(28.0);
            let title_y = h * 0.18;
            frame.text(TextLabel {
                text: celeb.pack_name.to_string(),
                rect: [0.0, title_y, w, title_font * 1.5],
                font_px: Some(title_font),
                color: color::CHAMPAGNE,
                align: TextAlign::Center,
                ..Default::default()
            });

            match celeb.phase {
                CelebPhase::Closeup => {
                    // ── Closeup: large pack box centered on screen ────
                    // Rendered via PackPlacement so it gets the foil
                    // material + texture. Gently bobs in place.
                    let box_h = h * 0.28;
                    let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                    let box_d = box_h * 0.10;
                    let t = celeb.started_at.elapsed().as_secs_f32();
                    // Calm, slow bob using incommensurate frequencies so
                    // the motion feels organic, not mechanical.
                    let bob_x = (t * 0.7).sin() * h * 0.008;
                    let bob_y = (t * 0.5).sin() * h * 0.006;
                    let bob_rx = (t * 0.6).sin() * 2.5; // degrees
                    let bob_ry = (t * 0.8).cos() * 3.0; // degrees
                    frame.object3d_batch(vec![Object3d {
                        pos: [
                            w * 0.5 + bob_x,
                            h * self.positions.celeb_pack_closeup.ny + bob_y,
                            layout.mm(self.positions.celeb_pack_closeup.lift_mm) + box_h * 0.5,
                        ],
                        extents: [box_w, box_d, box_h],
                        rotation: rot_ry_rx_deg(bob_rx, bob_ry),
                        color: celeb.pack_kind.foil_tint(),
                        kind: Object3dKind::Pack {
                            kind: celeb.pack_kind,
                            pick_id: None,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.celebrations.pack_closeup"),
                    }]);

                    // "Click to open" prompt at the bottom.
                    let prompt_font = (h * 0.028).max(18.0);
                    let prompt_y = h * 0.88;
                    let t = celeb.started_at.elapsed().as_secs_f32();
                    let pulse_alpha = 0.5 + 0.5 * (t * 3.0).sin();
                    frame.text(TextLabel {
                        text: "Click or press confirm to open".to_string(),
                        rect: [0.0, prompt_y, w, prompt_font * 1.5],
                        font_px: Some(prompt_font),
                        color: [1.0, 1.0, 1.0, pulse_alpha],
                        align: TextAlign::Center,
                        ..Default::default()
                    });
                }
                CelebPhase::Reveal => {
                    // ── Reveal: tiles flying out into a row ───────────
                    let tile_size = h * 0.13;
                    let gap = tile_size * 0.25;
                    let total_w = n as f32 * tile_size + (n.saturating_sub(1)) as f32 * gap;
                    let row_x0 = (w - total_w) * 0.5 + w * self.positions.celeb_pack_reveal.nx;
                    let row_py = h * self.positions.celeb_pack_reveal.ny;
                    let row_lift = layout.mm(self.positions.celeb_pack_reveal.lift_mm);
                    let src_px = w * 0.5;
                    let src_py = h * self.positions.celeb_pack_closeup.ny;
                    let src_lift = row_lift + h * 0.15;
                    let row_rx = self.positions.celeb_pack_reveal.rx_deg.to_radians()
                        + 60.0_f32.to_radians();
                    let row_ry = self.positions.celeb_pack_reveal.ry_deg.to_radians();
                    let row_rz =
                        self.positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI;

                    let mut placements = Vec::with_capacity(n);
                    for i in 0..n {
                        let t = celeb.tile_progress(i);
                        let ease = 1.0 - (1.0 - t).powi(3);

                        let dest_px = row_x0 + i as f32 * (tile_size + gap) + tile_size * 0.5;
                        let px = src_px + (dest_px - src_px) * ease;
                        let py = src_py + (row_py - src_py) * ease;
                        let lift = src_lift + (row_lift - src_lift) * ease;
                        let scale = 0.3 + 0.7 * ease;

                        placements.push(ShowcaseTilePlacement {
                            tile: celeb.tiles[i],
                            center_pos: [px, py, lift],
                            rotation: [row_rx, row_ry, row_rz],
                            scale,
                            size_px: tile_size,
                            brightness: 1.0,
                            selected: false,
                            hovered: false,
                            outline: false,
                            glow: false,
                            glow_color: None,
                            pick_id: None,
                        });
                    }

                    frame
                        .cmds
                        .push(crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(
                            placements,
                        ));
                    // Pack-reveal tiles render through `ShowcaseTileBatch`,
                    // which has no per-tile `arrange_name`, so committed
                    // placement values (read above as `nx/ny/lift_mm/rx/ry/rz`)
                    // are the only way to nudge them. Select
                    // `shop.celebrations.pack_reveal` from the arrange-mode
                    // hierarchy via Tab — there is no clickable mesh anchor.

                    // Dismiss prompt — pinned near the bottom.
                    if celeb.fully_settled() {
                        let prompt_font = (h * 0.028).max(18.0);
                        let prompt_y = h * 0.88;
                        let pulse_alpha = 0.5 + 0.5 * ((celeb.elapsed() * 3.0).sin());
                        frame.text(TextLabel {
                            text: "Click or press confirm to continue".to_string(),
                            rect: [0.0, prompt_y, w, prompt_font * 1.5],
                            font_px: Some(prompt_font),
                            color: [1.0, 1.0, 1.0, pulse_alpha],
                            align: TextAlign::Center,
                            ..Default::default()
                        });
                    }
                }
            }

            // Block all shop buttons so clicks go to the celebration.
            buttons.clear();
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));
        }

        // Zodiac level-up feedback: floating text + particles.
        let now = Instant::now();
        let popup_scale = w.min(h) / 1080.0;
        let glyph_placements = self.score_popups.placements(now, popup_scale);
        if !glyph_placements.is_empty() {
            frame.object3d_batch(glyph_placements);
        }
        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance { rect, color });
        }

        frame.buttons = buttons;
        frame.window_title = format!(
            "Mahjuro — Shop (Round {}) — Gold: {}",
            self.came_from_round, shop.gold
        );

        frame
    }
}
