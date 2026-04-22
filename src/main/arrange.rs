/// Raw arrange-mode input accumulated per frame, in pixel / millimetre /
/// degree units. Normalised into [`ArrangeDelta`] by the layout-apply
/// entry point once the window size is known.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArrangeInput {
    pub delta_px: f32,
    pub delta_py: f32,
    pub delta_lift: f32,
    pub delta_rx_deg: f32,
    pub delta_ry_deg: f32,
    pub delta_rz_deg: f32,
}

pub fn apply_arrange_to_layout(
    name: &str,
    input: ArrangeInput,
    window_w: f32,
    window_h: f32,
    scene: &mut crate::Scene,
) {
    use crate::ui::placement::{ArrangeDelta, apply_arrange};
    use crate::ui::scene_layout::{
        save_collection_positions, save_gameplay_positions, save_shop_positions,
        save_start_screen_positions, save_tutorial_positions,
    };

    let delta = ArrangeDelta {
        dnx: input.delta_px / window_w,
        dny: input.delta_py / window_h,
        d_lift_mm: input.delta_lift * crate::ui::scene_layout::HFRAC_TO_MM / window_w,
        d_rx_deg: input.delta_rx_deg,
        d_ry_deg: input.delta_ry_deg,
        d_rz_deg: input.delta_rz_deg,
    };

    let (matched, save_result): (bool, Option<anyhow::Result<()>>) = match scene {
        crate::Scene::Gameplay(gp) => {
            let p = &mut gp.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_gameplay_positions(p)))
        }
        crate::Scene::Shop(shop) => {
            let p = &mut shop.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_shop_positions(p)))
        }
        crate::Scene::Collection(c) => {
            let p = &mut c.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_collection_positions(p)))
        }
        crate::Scene::StartScreen(s) => {
            let p = &mut s.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_start_screen_positions(p)))
        }
        crate::Scene::TutorialCampaign(t) => {
            let p = &mut t.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_tutorial_positions(p)))
        }
        _ => (false, None),
    };

    if matched {
        if let Some(Err(e)) = save_result {
            log::error!("[Arrange] Failed to save layout: {e}");
        } else {
            log::info!("[Arrange] Saved layout (object: {name})");
        }
    } else {
        log::info!("[Arrange] '{name}' has no layout field mapping — clipboard only");
    }
}

pub fn sample_arrange_placement(
    name: &str,
    scene: &crate::Scene,
) -> Option<crate::ui::placement::Placement> {
    use crate::ui::placement::ArrangeTarget;
    match scene {
        crate::Scene::Gameplay(gp) => gp.positions.placement(name).copied(),
        crate::Scene::Shop(shop) => shop.positions.placement(name).copied(),
        crate::Scene::Collection(c) => c.positions.placement(name).copied(),
        crate::Scene::StartScreen(s) => s.positions.placement(name).copied(),
        crate::Scene::TutorialCampaign(t) => t.positions.placement(name).copied(),
        _ => None,
    }
}

pub fn reset_arrange_to_default(name: &str, scene: &mut crate::Scene) {
    use crate::ui::placement::reset_arrange;
    use crate::ui::scene_layout::{
        save_collection_positions, save_gameplay_positions, save_shop_positions,
        save_start_screen_positions, save_tutorial_positions,
    };

    let (matched, save_result): (bool, Option<anyhow::Result<()>>) = match scene {
        crate::Scene::Gameplay(gp) => {
            let p = &mut gp.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_gameplay_positions(p)))
        }
        crate::Scene::Shop(shop) => {
            let p = &mut shop.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_shop_positions(p)))
        }
        crate::Scene::Collection(c) => {
            let p = &mut c.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_collection_positions(p)))
        }
        crate::Scene::StartScreen(s) => {
            let p = &mut s.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_start_screen_positions(p)))
        }
        crate::Scene::TutorialCampaign(t) => {
            let p = &mut t.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_tutorial_positions(p)))
        }
        _ => (false, None),
    };

    if matched {
        if let Some(Err(e)) = save_result {
            log::error!("[Arrange] Failed to save layout after reset: {e}");
        } else {
            log::info!("[Arrange] Reset '{name}' to default");
        }
    } else {
        log::info!("[Arrange] '{name}' has no layout field mapping — cannot reset");
    }
}

pub fn collect_committed_rotations(
    scene: &crate::Scene,
) -> std::collections::HashMap<String, [f32; 3]> {
    use crate::ui::placement::{ArrangeTarget, all_leaf_names};
    type PlacementLookup<'a> = Box<dyn Fn(&str) -> Option<crate::ui::placement::Placement> + 'a>;
    let mut out = std::collections::HashMap::new();
    let (hierarchy, lookup): (&'static [crate::ui::placement::Node], PlacementLookup<'_>) =
        match scene {
            crate::Scene::Gameplay(gp) => (
                gp.positions.hierarchy(),
                Box::new(move |n| gp.positions.placement(n).copied()),
            ),
            crate::Scene::Shop(shop) => (
                shop.positions.hierarchy(),
                Box::new(move |n| shop.positions.placement(n).copied()),
            ),
            crate::Scene::Collection(c) => (
                c.positions.hierarchy(),
                Box::new(move |n| c.positions.placement(n).copied()),
            ),
            crate::Scene::StartScreen(s) => (
                s.positions.hierarchy(),
                Box::new(move |n| s.positions.placement(n).copied()),
            ),
            crate::Scene::TutorialCampaign(t) => (
                t.positions.hierarchy(),
                Box::new(move |n| t.positions.placement(n).copied()),
            ),
            _ => return out,
        };
    for name in all_leaf_names(hierarchy) {
        if let Some(p) = lookup(name)
            && (p.rx_deg != 0.0 || p.ry_deg != 0.0 || p.rz_deg != 0.0)
        {
            out.insert(name.to_string(), [p.rx_deg, p.ry_deg, p.rz_deg]);
        }
    }
    out
}

pub struct HierarchyEntry {
    pub name: &'static str,
    pub label: &'static str,
    pub depth: usize,
    pub is_group: bool,
}

pub fn arrange_hierarchy_flat(scene: &crate::Scene) -> Vec<HierarchyEntry> {
    use crate::ui::placement::{ArrangeTarget, Node};

    fn walk(nodes: &'static [Node], depth: usize, out: &mut Vec<HierarchyEntry>) {
        for n in nodes {
            match n {
                Node::Leaf { name, label } => out.push(HierarchyEntry {
                    name,
                    label,
                    depth,
                    is_group: false,
                }),
                Node::Group {
                    name,
                    label,
                    children,
                } => {
                    out.push(HierarchyEntry {
                        name,
                        label,
                        depth,
                        is_group: true,
                    });
                    walk(children, depth + 1, out);
                }
            }
        }
    }

    let hierarchy: &'static [Node] = match scene {
        crate::Scene::Shop(s) => s.positions.hierarchy(),
        crate::Scene::Gameplay(g) => g.positions.hierarchy(),
        crate::Scene::Collection(c) => c.positions.hierarchy(),
        crate::Scene::StartScreen(s) => s.positions.hierarchy(),
        crate::Scene::TutorialCampaign(t) => t.positions.hierarchy(),
        _ => &[],
    };
    let mut out = Vec::new();
    walk(hierarchy, 0, &mut out);
    out
}
