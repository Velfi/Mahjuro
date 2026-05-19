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
        save_collection_positions, save_gameplay_positions, save_main_menu_exterior_positions,
        save_shop_positions, save_tile_select_positions, save_tutorial_positions,
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
        crate::Scene::Shop(s) => {
            let p = &mut s.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_shop_positions(p)))
        }
        crate::Scene::Collection(c) => {
            let p = &mut c.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_collection_positions(p)))
        }
        crate::Scene::MainMenuExterior(s) => {
            let p = &mut s.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_main_menu_exterior_positions(p)))
        }
        crate::Scene::TutorialCampaign(t) => {
            let p = &mut t.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_tutorial_positions(p)))
        }
        crate::Scene::Showcase(s) => match &mut s.presenter {
            crate::scenes::ShowcasePresenter::TilePack(t) => {
                let p = &mut t.positions;
                let ok = apply_arrange(p, name, delta);
                (ok, ok.then(|| save_shop_positions(p)))
            }
            _ => (false, None),
        },
        crate::Scene::TileSelect(s) => {
            let p = &mut s.positions;
            let ok = apply_arrange(p, name, delta);
            (ok, ok.then(|| save_tile_select_positions(p)))
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
        crate::Scene::Shop(s) => s.positions.placement(name).copied(),
        crate::Scene::Collection(c) => c.positions.placement(name).copied(),
        crate::Scene::MainMenuExterior(s) => s.positions.placement(name).copied(),
        crate::Scene::TutorialCampaign(t) => t.positions.placement(name).copied(),
        crate::Scene::Showcase(s) => match &s.presenter {
            crate::scenes::ShowcasePresenter::TilePack(t) => t.positions.placement(name).copied(),
            _ => None,
        },
        crate::Scene::TileSelect(s) => s.positions.placement(name).copied(),
        _ => None,
    }
}

pub fn reset_arrange_to_default(name: &str, scene: &mut crate::Scene) {
    use crate::ui::placement::reset_arrange;
    use crate::ui::scene_layout::{
        save_collection_positions, save_gameplay_positions, save_main_menu_exterior_positions,
        save_shop_positions, save_tile_select_positions, save_tutorial_positions,
    };

    let (matched, save_result): (bool, Option<anyhow::Result<()>>) = match scene {
        crate::Scene::Gameplay(gp) => {
            let p = &mut gp.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_gameplay_positions(p)))
        }
        crate::Scene::Shop(s) => {
            let p = &mut s.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_shop_positions(p)))
        }
        crate::Scene::Collection(c) => {
            let p = &mut c.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_collection_positions(p)))
        }
        crate::Scene::MainMenuExterior(s) => {
            let p = &mut s.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_main_menu_exterior_positions(p)))
        }
        crate::Scene::TutorialCampaign(t) => {
            let p = &mut t.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_tutorial_positions(p)))
        }
        crate::Scene::Showcase(s) => match &mut s.presenter {
            crate::scenes::ShowcasePresenter::TilePack(t) => {
                let p = &mut t.positions;
                let ok = reset_arrange(p, name);
                (ok, ok.then(|| save_shop_positions(p)))
            }
            _ => (false, None),
        },
        crate::Scene::TileSelect(s) => {
            let p = &mut s.positions;
            let ok = reset_arrange(p, name);
            (ok, ok.then(|| save_tile_select_positions(p)))
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
) -> rustc_hash::FxHashMap<String, [f32; 3]> {
    use crate::ui::placement::{ArrangeTarget, all_leaf_names};
    type PlacementLookup<'a> = Box<dyn Fn(&str) -> Option<crate::ui::placement::Placement> + 'a>;
    let mut out = rustc_hash::FxHashMap::default();
    let (hierarchy, lookup): (&'static [crate::ui::placement::Node], PlacementLookup<'_>) =
        match scene {
            crate::Scene::Gameplay(gp) => (
                gp.positions.hierarchy(),
                Box::new(move |n| gp.positions.placement(n).copied()),
            ),
            crate::Scene::Shop(s) => (
                s.positions.hierarchy(),
                Box::new(move |n| s.positions.placement(n).copied()),
            ),
            crate::Scene::Collection(c) => (
                c.positions.hierarchy(),
                Box::new(move |n| c.positions.placement(n).copied()),
            ),
            crate::Scene::MainMenuExterior(s) => (
                s.positions.hierarchy(),
                Box::new(move |n| s.positions.placement(n).copied()),
            ),
            crate::Scene::TutorialCampaign(t) => (
                t.positions.hierarchy(),
                Box::new(move |n| t.positions.placement(n).copied()),
            ),
            crate::Scene::Showcase(s) => match &s.presenter {
                crate::scenes::ShowcasePresenter::TilePack(t) => (
                    t.positions.hierarchy(),
                    Box::new(move |n| t.positions.placement(n).copied()),
                ),
                _ => return out,
            },
            crate::Scene::TileSelect(s) => (
                s.positions.hierarchy(),
                Box::new(move |n| s.positions.placement(n).copied()),
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
        crate::Scene::MainMenuExterior(s) => s.positions.hierarchy(),
        crate::Scene::TutorialCampaign(t) => t.positions.hierarchy(),
        crate::Scene::Showcase(s) => match &s.presenter {
            crate::scenes::ShowcasePresenter::TilePack(t) => t.positions.hierarchy(),
            _ => &[],
        },
        crate::Scene::TileSelect(s) => s.positions.hierarchy(),
        _ => &[],
    };
    let mut out = Vec::new();
    walk(hierarchy, 0, &mut out);
    out
}

/// Index of `name` in [`arrange_hierarchy_flat`], if it appears in the active scene.
pub fn arrange_hierarchy_index_of(flat: &[HierarchyEntry], name: &str) -> Option<usize> {
    flat.iter().position(|e| e.name == name)
}

/// Parent node in the flat list. `None` at the root tier.
pub fn arrange_hierarchy_parent(flat: &[HierarchyEntry], idx: usize) -> Option<usize> {
    let d = flat.get(idx)?.depth;
    if d == 0 {
        return None;
    }
    let target = d - 1;
    let mut j = idx;
    while j > 0 {
        j -= 1;
        if flat[j].depth == target {
            return Some(j);
        }
    }
    None
}

/// Direct child **indices** in the flat pre-order list (immediate children only).
fn arrange_hierarchy_direct_child_indices(flat: &[HierarchyEntry], parent_idx: usize) -> Vec<usize> {
    let n = flat.len();
    if parent_idx >= n || !flat[parent_idx].is_group {
        return Vec::new();
    }
    let base_depth = flat[parent_idx].depth;
    let mut out = Vec::new();
    let mut i = parent_idx + 1;
    while i < n && flat[i].depth > base_depth {
        if flat[i].depth == base_depth + 1 {
            out.push(i);
            let child_depth = flat[i].depth;
            i += 1;
            while i < n && flat[i].depth > child_depth {
                i += 1;
            }
        } else {
            // Should not happen in a well-formed hierarchy walk.
            i += 1;
        }
    }
    out
}

fn arrange_hierarchy_root_indices(flat: &[HierarchyEntry]) -> Vec<usize> {
    flat.iter()
        .enumerate()
        .filter(|(_, e)| e.depth == 0)
        .map(|(i, _)| i)
        .collect()
}

/// Previous / next **sibling** in the hierarchy (wraps). Roots (`depth == 0`) share
/// one virtual parent. Returns `None` if `idx` is absent or is an only child.
pub fn arrange_hierarchy_sibling_offset(flat: &[HierarchyEntry], idx: usize, next: bool) -> Option<usize> {
    let parent = arrange_hierarchy_parent(flat, idx);
    let siblings: Vec<usize> = match parent {
        Some(p) => arrange_hierarchy_direct_child_indices(flat, p),
        None => arrange_hierarchy_root_indices(flat),
    };
    let pos = siblings.iter().position(|&x| x == idx)?;
    if siblings.len() <= 1 {
        return None;
    }
    let len = siblings.len();
    let p = if next {
        (pos + 1) % len
    } else {
        (pos + len - 1) % len
    };
    Some(siblings[p])
}

/// First direct child of a group. `None` for leaves or empty groups.
pub fn arrange_hierarchy_first_child(flat: &[HierarchyEntry], idx: usize) -> Option<usize> {
    let ch = arrange_hierarchy_direct_child_indices(flat, idx);
    ch.into_iter().next()
}

#[cfg(test)]
mod hierarchy_nav_tests {
    use super::*;

    fn e(name: &'static str, depth: usize, is_group: bool) -> HierarchyEntry {
        HierarchyEntry {
            name,
            label: name,
            depth,
            is_group,
        }
    }

    /// Mirrors a small tree: A(g) -> B(g) -> C(leaf), D(leaf); A -> E(leaf).
    fn sample_flat() -> Vec<HierarchyEntry> {
        vec![
            e("a", 0, true),
            e("b", 1, true),
            e("c", 2, false),
            e("d", 2, false),
            e("e", 1, false),
        ]
    }

    #[test]
    fn parent_and_first_child() {
        let flat = sample_flat();
        assert_eq!(arrange_hierarchy_parent(&flat, 0), None);
        assert_eq!(arrange_hierarchy_first_child(&flat, 0), Some(1));
        assert_eq!(arrange_hierarchy_parent(&flat, 1), Some(0));
        assert_eq!(arrange_hierarchy_first_child(&flat, 1), Some(2));
        assert_eq!(arrange_hierarchy_parent(&flat, 2), Some(1));
        assert_eq!(arrange_hierarchy_first_child(&flat, 2), None);
        assert_eq!(arrange_hierarchy_parent(&flat, 4), Some(0));
    }

    #[test]
    fn siblings_wrap() {
        let flat = sample_flat();
        assert_eq!(arrange_hierarchy_sibling_offset(&flat, 2, true), Some(3));
        assert_eq!(arrange_hierarchy_sibling_offset(&flat, 3, false), Some(2));
        assert_eq!(arrange_hierarchy_sibling_offset(&flat, 1, true), Some(4));
        assert_eq!(arrange_hierarchy_sibling_offset(&flat, 4, true), Some(1));
        assert_eq!(arrange_hierarchy_sibling_offset(&flat, 0, true), None);
    }
}
