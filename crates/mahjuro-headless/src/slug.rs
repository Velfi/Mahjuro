/// Compact slug: lowercase, no spaces/underscores/hyphens.
#[cfg(feature = "screenshot")]
pub(crate) fn normalize_slug(slug: &str) -> String {
    slug.trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

/// Map `--boss` to [`OrdealKind`].
#[cfg(feature = "screenshot")]
pub(crate) fn parse_ordeal_slug(slug: &str) -> anyhow::Result<mahjuro::core::ordeal::OrdealKind> {
    let normalized = normalize_slug(slug);
    let normalize_name = |s: &str| {
        s.to_ascii_lowercase()
            .replace(['_', '-', ' ', '\''], "")
            .replace("the", "")
    };
    for def in mahjuro::core::ordeal::all_ordeals()
        .iter()
        .chain(mahjuro::core::ordeal::final_ordeals().iter())
    {
        if normalize_name(def.name) == normalized
            || format!("{:?}", def.kind).to_ascii_lowercase() == normalized
        {
            return Ok(def.kind);
        }
    }
    anyhow::bail!("unknown --boss '{slug}'");
}

#[cfg(feature = "screenshot")]
pub(crate) fn parse_zodiac_slug(slug: &str) -> anyhow::Result<mahjuro::core::zodiac::ZodiacKind> {
    let normalized = normalize_slug(slug);
    for z in mahjuro::core::zodiac::ZodiacKind::all() {
        if z.slug() == normalized || z.name().to_ascii_lowercase().replace(' ', "") == normalized {
            return Ok(*z);
        }
    }
    anyhow::bail!("unknown --zodiac '{slug}'");
}

#[cfg(feature = "screenshot")]
pub(crate) fn parse_tile_pack_slug(
    slug: &str,
) -> anyhow::Result<mahjuro::core::tile_pack::TilePackKind> {
    use mahjuro::core::tile_pack::TilePackKind;
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
) -> anyhow::Result<mahjuro::render::room_gi_bake::RoomGiRoom> {
    match slug.trim().to_ascii_lowercase().as_str() {
        "shop" => Ok(mahjuro::render::room_gi_bake::RoomGiRoom::Shop),
        "hallway" | "pick_chamber" => Ok(mahjuro::render::room_gi_bake::RoomGiRoom::Hallway),
        "archive" | "collection" => Ok(mahjuro::render::room_gi_bake::RoomGiRoom::Archive),
        "main_menu" | "main-menu" | "main_menu_exterior" => {
            Ok(mahjuro::render::room_gi_bake::RoomGiRoom::MainMenu)
        }
        "staircase" | "stairway" => Ok(mahjuro::render::room_gi_bake::RoomGiRoom::Stairway),
        "gameplay" => Ok(mahjuro::render::room_gi_bake::RoomGiRoom::Gameplay),
        "shadow_test_room" | "shadow-test-room" | "shadowtestroom" => {
            Ok(mahjuro::render::room_gi_bake::RoomGiRoom::ShadowTestRoom)
        }
        other => anyhow::bail!(
            "unknown room '{other}' (try shop, hallway, stairway, archive, main_menu, gameplay, shadow_test_room)"
        ),
    }
}

const SHADOW_BAKE_ROOMS: &[mahjuro::render::room_gi_bake::RoomGiRoom] = &[
    mahjuro::render::room_gi_bake::RoomGiRoom::Shop,
    mahjuro::render::room_gi_bake::RoomGiRoom::Hallway,
    mahjuro::render::room_gi_bake::RoomGiRoom::Archive,
    mahjuro::render::room_gi_bake::RoomGiRoom::MainMenu,
    mahjuro::render::room_gi_bake::RoomGiRoom::Stairway,
    mahjuro::render::room_gi_bake::RoomGiRoom::Gameplay,
];

pub(crate) fn resolve_lightmap_bake_rooms(
    slugs: &[String],
) -> anyhow::Result<Vec<mahjuro::render::room_gi_bake::RoomGiRoom>> {
    if slugs.is_empty() {
        return Ok(mahjuro::render::room_gi_bake::RoomGiRoom::ALL.to_vec());
    }
    resolve_explicit_bake_rooms(slugs)
}

pub(crate) fn resolve_shadow_bake_rooms(
    slugs: &[String],
) -> anyhow::Result<Vec<mahjuro::render::room_gi_bake::RoomGiRoom>> {
    if slugs.is_empty() {
        return Ok(SHADOW_BAKE_ROOMS.to_vec());
    }
    Ok(resolve_explicit_bake_rooms(slugs)?
        .into_iter()
        .filter(|room| SHADOW_BAKE_ROOMS.contains(room))
        .collect())
}

fn resolve_explicit_bake_rooms(
    slugs: &[String],
) -> anyhow::Result<Vec<mahjuro::render::room_gi_bake::RoomGiRoom>> {
    let mut rooms = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let room = parse_bake_room_slug(slug)?;
        if !rooms.contains(&room) {
            rooms.push(room);
        }
    }
    Ok(rooms)
}
