/// Compact slug: lowercase, no spaces/underscores/hyphens.
pub(crate) fn normalize_slug(slug: &str) -> String {
    slug.trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

/// Map `--boss` to [`OrdealKind`].
pub(crate) fn parse_ordeal_slug(slug: &str) -> anyhow::Result<crate::core::ordeal::OrdealKind> {
    let normalized = normalize_slug(slug);
    let normalize_name = |s: &str| {
        s.to_ascii_lowercase()
            .replace(['_', '-', ' ', '\''], "")
            .replace("the", "")
    };
    for def in crate::core::ordeal::all_ordeals()
        .iter()
        .chain(crate::core::ordeal::final_ordeals().iter())
    {
        if normalize_name(def.name) == normalized
            || format!("{:?}", def.kind).to_ascii_lowercase() == normalized
        {
            return Ok(def.kind);
        }
    }
    anyhow::bail!("unknown --boss '{slug}'");
}

pub(crate) fn parse_zodiac_slug(slug: &str) -> anyhow::Result<crate::core::zodiac::ZodiacKind> {
    let normalized = normalize_slug(slug);
    for z in crate::core::zodiac::ZodiacKind::all() {
        if z.slug() == normalized || z.name().to_ascii_lowercase().replace(' ', "") == normalized {
            return Ok(*z);
        }
    }
    anyhow::bail!("unknown --zodiac '{slug}'");
}

pub(crate) fn parse_tile_pack_slug(
    slug: &str,
) -> anyhow::Result<crate::core::tile_pack::TilePackKind> {
    use crate::core::tile_pack::TilePackKind;
    let n = normalize_slug(slug);
    for &k in TilePackKind::all() {
        let debug_s = format!("{:?}", k).to_ascii_lowercase();
        let compact: String = k
            .name()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if n == debug_s || n == compact {
            return Ok(k);
        }
    }
    anyhow::bail!("unknown --pack '{slug}' (try honors, terminals, flowers, souzu, pinzu, manzu)");
}

pub(crate) fn parse_bake_room_slug(
    slug: &str,
) -> anyhow::Result<crate::render::room_gi_bake::RoomGiRoom> {
    match slug.trim().to_ascii_lowercase().as_str() {
        "shop" => Ok(crate::render::room_gi_bake::RoomGiRoom::Shop),
        "hallway" | "pick_chamber" => Ok(crate::render::room_gi_bake::RoomGiRoom::Hallway),
        "archive" | "collection" => Ok(crate::render::room_gi_bake::RoomGiRoom::Archive),
        "main_menu" | "main-menu" | "main_menu_exterior" => {
            Ok(crate::render::room_gi_bake::RoomGiRoom::MainMenu)
        }
        "staircase" | "stairway" => Ok(crate::render::room_gi_bake::RoomGiRoom::Staircase),
        "gameplay" => Ok(crate::render::room_gi_bake::RoomGiRoom::Gameplay),
        other => anyhow::bail!(
            "unknown room '{other}' (use shop, hallway, staircase, archive, main_menu, or gameplay)"
        ),
    }
}
