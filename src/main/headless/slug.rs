/// Compact slug: lowercase, no spaces/underscores/hyphens.
pub(crate) fn normalize_slug(slug: &str) -> String {
    slug.trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

/// Map `--boss` to [`BossKind`].
pub(crate) fn parse_boss_slug(slug: &str) -> anyhow::Result<crate::core::boss::BossKind> {
    let normalized = normalize_slug(slug);
    let normalize_name = |s: &str| {
        s.to_ascii_lowercase()
            .replace(['_', '-', ' ', '\''], "")
            .replace("the", "")
    };
    for def in crate::core::boss::all_bosses()
        .iter()
        .chain(crate::core::boss::final_bosses().iter())
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
    anyhow::bail!(
        "unknown --pack '{slug}' (try honors, terminals, flowers, bamboo_grove, coin_cache, scroll_library)"
    );
}

pub(crate) fn parse_bake_room_slug(slug: &str) -> anyhow::Result<crate::render::room_gi_bake::RoomGiRoom> {
    match slug.trim().to_ascii_lowercase().as_str() {
        "shop" => Ok(crate::render::room_gi_bake::RoomGiRoom::Shop),
        "hallway" | "pick_blind" => Ok(crate::render::room_gi_bake::RoomGiRoom::Hallway),
        "archive" | "collection" => Ok(crate::render::room_gi_bake::RoomGiRoom::Archive),
        "main_menu" | "main-menu" | "main_menu_exterior" => {
            Ok(crate::render::room_gi_bake::RoomGiRoom::MainMenu)
        }
        other => anyhow::bail!("unknown room '{other}' (use shop, hallway, archive, or main_menu)"),
    }
}
