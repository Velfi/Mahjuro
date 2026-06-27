use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use mahjuro::core::hand::{DetectedMeld, MeldKind, decomposition_canonical_key};
use mahjuro::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle,
};
use mahjuro::core::rules::RuleModifier;
use mahjuro::core::scoring::{ScoreBreakdown, ScoreStep, StepKind, score_sets_with_original};
use mahjuro::core::structure::StructureTriggerMeta;
use mahjuro::core::tile::{Suit, Tile};
use mahjuro::core::yaku::YakuKind;
use mahjuro::core::zodiac::YakuLevels;
use rand::RngExt;
use rand::SeedableRng;
use rand::prelude::IndexedRandom;
use rand::rngs::StdRng;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    name = "mahjuro-score-fixture-export",
    about = "Export random Mahjuro scoring fixtures from the reference Rust engine"
)]
struct Args {
    /// Number of fixtures to emit.
    #[arg(long, default_value_t = 10_000)]
    count: usize,

    /// Deterministic RNG seed.
    #[arg(long, default_value_t = 0x4d41_484a_5552_4f01)]
    seed: u64,

    /// Optional output path. Defaults to stdout.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Emit pretty JSON instead of compact JSON.
    #[arg(long)]
    pretty: bool,

    /// Minimum meld count in a generated structure.
    #[arg(long, default_value_t = 1)]
    min_melds: usize,

    /// Maximum meld count in a generated structure.
    #[arg(long, default_value_t = 5)]
    max_melds: usize,

    /// Include boss/rule modifier coverage in the generated corpus.
    #[arg(long)]
    include_rules: bool,

    /// Include dora indicator faces in the generated corpus.
    #[arg(long)]
    include_dora: bool,

    /// Include random yaku levels in the generated corpus.
    #[arg(long)]
    include_yaku_levels: bool,

    /// Include deterministic scoring relic loadouts and relic counter state.
    #[arg(long)]
    include_relics: bool,

    /// Force every fixture to include relics; implies --include-relics.
    #[arg(long)]
    relic_only: bool,

    /// Every Nth fixture is a randomized known-yaku archetype. Use 0 for pure random melds.
    #[arg(long, default_value_t = 7)]
    special_every: usize,
}

#[derive(Debug, Serialize)]
struct Corpus {
    schema: &'static str,
    schema_version: u32,
    generator: GeneratorMeta,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Serialize)]
struct GeneratorMeta {
    engine: &'static str,
    seed: u64,
    requested_count: usize,
    min_melds: usize,
    max_melds: usize,
    include_rules: bool,
    include_dora: bool,
    include_yaku_levels: bool,
    include_relics: bool,
    relic_only: bool,
    special_every: usize,
}

#[derive(Debug, Serialize)]
struct Fixture {
    id: String,
    seed: u64,
    index: usize,
    archetype: String,
    melds: Vec<MeldFixture>,
    tiles: Vec<TileFixture>,
    rules: Vec<String>,
    context: ContextFixture,
    canonical_key: serde_json::Value,
    score: ScoreFixture,
}

#[derive(Debug, Serialize)]
struct MeldFixture {
    kind: String,
    tile_ids: Vec<u32>,
    labels: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TileFixture {
    id: u32,
    suit: String,
    rank: u8,
    label: String,
}

#[derive(Debug, Serialize)]
struct ContextFixture {
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    dora_faces: Vec<FaceFixture>,
    yaku_levels: BTreeMap<String, u32>,
    relics: Vec<String>,
    debuffed_relics: Vec<String>,
    relic_counters: BTreeMap<String, i32>,
    scored_last_turn: bool,
    plays_used: u32,
    is_final_play: bool,
    yen: i32,
    total_score: u64,
    inject_chicken_if_no_yaku: bool,
}

#[derive(Debug, Serialize)]
struct FaceFixture {
    suit: String,
    rank: u8,
}

#[derive(Debug, Serialize)]
struct ScoreFixture {
    base_fu: i32,
    base_points: i32,
    final_fu: i32,
    final_han: f64,
    total: u64,
    flower_yen: i32,
    detected_yaku: Vec<String>,
    base_steps: Vec<StepFixture>,
    steps: Vec<StepFixture>,
    scored_meld_kinds: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StepFixture {
    source: String,
    kind: String,
    tile_ids: Vec<u32>,
    running_fu: i32,
    running_han: f64,
    running_total: u64,
}

#[derive(Clone, Copy)]
struct Face {
    suit: Suit,
    rank: u8,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.count == 0 {
        return Err(anyhow!("--count must be greater than zero"));
    }
    if args.min_melds == 0 || args.max_melds == 0 || args.min_melds > args.max_melds {
        return Err(anyhow!("expected 1 <= --min-melds <= --max-melds"));
    }
    if args.max_melds > 5 {
        return Err(anyhow!(
            "--max-melds must be <= 5 for standard structure fixtures"
        ));
    }
    let include_relics = args.include_relics || args.relic_only;

    let mut rng = StdRng::seed_from_u64(args.seed);
    let mut fixtures = Vec::with_capacity(args.count);
    for index in 0..args.count {
        fixtures.push(generate_fixture(&args, &mut rng, index)?);
    }

    let corpus = Corpus {
        schema: if include_relics {
            "mahjuro.relic_score_fixture_corpus"
        } else {
            "mahjuro.score_fixture_corpus"
        },
        schema_version: 1,
        generator: GeneratorMeta {
            engine: "mahjuro-rust",
            seed: args.seed,
            requested_count: args.count,
            min_melds: args.min_melds,
            max_melds: args.max_melds,
            include_rules: args.include_rules,
            include_dora: args.include_dora,
            include_yaku_levels: args.include_yaku_levels,
            include_relics,
            relic_only: args.relic_only,
            special_every: args.special_every,
        },
        fixtures,
    };

    match args.out {
        Some(path) => {
            let file = File::create(&path)
                .with_context(|| format!("create fixture corpus {}", path.display()))?;
            write_json(BufWriter::new(file), &corpus, args.pretty)?;
        }
        None => {
            let stdout = io::stdout();
            write_json(stdout.lock(), &corpus, args.pretty)?;
        }
    }
    Ok(())
}

fn write_json(mut writer: impl Write, corpus: &Corpus, pretty: bool) -> Result<()> {
    if pretty {
        serde_json::to_writer_pretty(&mut writer, corpus).context("serialize pretty corpus")?;
    } else {
        serde_json::to_writer(&mut writer, corpus).context("serialize corpus")?;
    }
    writer.write_all(b"\n").context("finish corpus JSON")?;
    Ok(())
}

fn generate_fixture(args: &Args, rng: &mut StdRng, index: usize) -> Result<Fixture> {
    let target_meld_count = rng.random_range(args.min_melds..=args.max_melds);
    let mut attempts = 0usize;
    loop {
        attempts += 1;
        if attempts > 10_000 {
            return Err(anyhow!(
                "failed to generate fixture {index} after {attempts} attempts"
            ));
        }

        let use_special = args.special_every > 0 && index % args.special_every == 0;
        let (archetype, rules, generated) = if use_special {
            let (name, tiles, melds) = random_special_structure(rng);
            (name, Vec::new(), Some((tiles, melds)))
        } else {
            let rules = random_rules(args.include_rules, rng);
            (
                "random".to_string(),
                rules.clone(),
                random_meld_structure(target_meld_count, &rules, rng),
            )
        };
        let Some((tiles, melds)) = generated else {
            continue;
        };

        let context = random_context(args, &tiles, &melds, rng);
        let relics = relic_state_from_context(&context);
        let ctx = score_context(&relics, &tiles, &context, melds.len());
        let score = score_sets_with_original(&tiles, &melds, &ctx, &rules, &tiles);

        return Ok(Fixture {
            id: format!("score-random-{:016x}-{index:06}", args.seed),
            seed: args.seed,
            index,
            archetype,
            melds: melds.iter().map(|m| meld_fixture(&tiles, m)).collect(),
            tiles: tiles.iter().copied().map(tile_fixture).collect(),
            rules: rules.iter().copied().map(rule_name).collect(),
            context: context_fixture(&context),
            canonical_key: serde_json::to_value(decomposition_canonical_key(&tiles, &melds))
                .context("serialize canonical decomposition key")?,
            score: score_fixture(&score),
        });
    }
}

fn score_context<'a>(
    relics: &'a RelicState,
    tiles: &'a [Tile],
    context: &'a GeneratedContext,
    meld_count: usize,
) -> ScoreContext<'a> {
    ScoreContext {
        relic: ScoreRelicBundle {
            roster: relics,
            counters: context.relic_counters.clone(),
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: tiles,
        },
        round: ScoreRoundBundle {
            scored_last_turn: context.scored_last_turn,
            plays_used: context.plays_used,
            round_wind: context.round_wind,
            bonus_round_wind: context.bonus_round_wind,
            played_yaku_this_round: Vec::new(),
            is_final_play: context.is_final_play,
        },
        pattern: ScorePatternBundle {
            dora_faces: context.dora_faces.clone(),
            available_yaku: Vec::new(),
            yaku_levels: context.yaku_levels.clone(),
        },
        economy: ScoreEconomyBundle {
            yen: context.yen,
            total_score: context.total_score,
        },
        structure: Some(StructureTriggerMeta {
            meld_count: meld_count as u32,
            inject_chicken_if_no_yaku: true,
        }),
    }
}

#[derive(Clone)]
struct GeneratedContext {
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    dora_faces: Vec<(Suit, u8)>,
    yaku_levels: Option<YakuLevels>,
    relics: Vec<RelicId>,
    debuffed_relics: Vec<RelicId>,
    relic_counters: BTreeMap<RelicId, i32>,
    scored_last_turn: bool,
    plays_used: u32,
    is_final_play: bool,
    yen: i32,
    total_score: u64,
}

fn random_context(
    args: &Args,
    tiles: &[Tile],
    melds: &[DetectedMeld],
    rng: &mut StdRng,
) -> GeneratedContext {
    let round_wind = Some(rng.random_range(1..=4));
    let bonus_round_wind = if rng.random_range(0..10) == 0 {
        let mut wind = rng.random_range(1..=4);
        while Some(wind) == round_wind {
            wind = rng.random_range(1..=4);
        }
        Some(wind)
    } else {
        None
    };
    let dora_faces = if args.include_dora {
        random_dora_faces(tiles, rng)
    } else {
        Vec::new()
    };
    let yaku_levels = if args.include_yaku_levels {
        Some(random_yaku_levels(
            tiles,
            melds,
            round_wind,
            bonus_round_wind,
            rng,
        ))
    } else {
        None
    };
    let include_relics = args.include_relics || args.relic_only;
    let (relics, debuffed_relics, relic_counters) = if include_relics {
        random_relic_context(rng)
    } else {
        (Vec::new(), Vec::new(), BTreeMap::new())
    };
    GeneratedContext {
        round_wind,
        bonus_round_wind,
        dora_faces,
        yaku_levels,
        relics,
        debuffed_relics,
        relic_counters,
        scored_last_turn: include_relics && rng.random_range(0..2) == 0,
        plays_used: if include_relics {
            rng.random_range(0..=4)
        } else {
            1
        },
        is_final_play: include_relics && rng.random_range(0..4) == 0,
        yen: if include_relics {
            rng.random_range(0..=24)
        } else {
            0
        },
        total_score: if include_relics {
            rng.random_range(0..=50_000)
        } else {
            0
        },
    }
}

fn relic_state_from_context(context: &GeneratedContext) -> RelicState {
    let mut state = RelicState {
        active: context.relics.clone(),
        max_slots: context.relics.len().max(5),
        ..Default::default()
    };
    state.set_debuffed(context.debuffed_relics.clone());
    state
}

fn random_dora_faces(tiles: &[Tile], rng: &mut StdRng) -> Vec<(Suit, u8)> {
    let mut out = Vec::new();
    let count = rng.random_range(0..=3);
    for _ in 0..count {
        if let Some(tile) = tiles.choose(rng) {
            out.push((tile.suit, tile.rank));
        }
    }
    out
}

fn random_yaku_levels(
    tiles: &[Tile],
    melds: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    rng: &mut StdRng,
) -> YakuLevels {
    let mut levels = YakuLevels::default();
    let detected = mahjuro::core::yaku::detect_yaku_with_wind(
        tiles,
        melds,
        round_wind,
        bonus_round_wind,
        None,
    );
    for yaku in detected {
        if rng.random_range(0..3) == 0 {
            let bumps = rng.random_range(1..=3);
            for _ in 0..bumps {
                levels.level_up(yaku);
            }
        }
    }
    levels
}

fn random_relic_context(rng: &mut StdRng) -> (Vec<RelicId>, Vec<RelicId>, BTreeMap<RelicId, i32>) {
    let pool = scoring_relic_pool();
    let count = rng.random_range(1..=5);
    let mut relics = Vec::new();
    for _ in 0..count {
        relics.push(*pool.choose(rng).expect("scoring relic pool is non-empty"));
    }

    if rng.random_range(0..6) == 0 && relics.len() >= 2 {
        relics[0] = RelicId::MirrorTile;
    }
    if rng.random_range(0..6) == 0 && relics.len() >= 2 {
        relics[0] = RelicId::ShadowHand;
    }

    let mut debuffed = Vec::new();
    for &id in &relics {
        if rng.random_range(0..20) == 0 {
            debuffed.push(id);
        }
    }

    let mut counters = BTreeMap::new();
    for &id in &relics {
        let value = match id {
            RelicId::Snowball => rng.random_range(0..=15),
            RelicId::TilePolisher => rng.random_range(0..=180),
            RelicId::RiverRunner => rng.random_range(0..=180),
            RelicId::XxxlEgg => rng.random_range(0..=3),
            RelicId::MeltingIce => rng.random_range(0..=120),
            RelicId::Taotie => rng.random_range(0..=240),
            RelicId::TeaCeremony => rng.random_range(0..=3),
            RelicId::MonarchButterfly => rng.random_range(0..=50_000),
            RelicId::SilkThread => rng.random_range(0..=160),
            RelicId::Humility => rng.random_range(0..=12),
            RelicId::Obsession => rng.random_range(0..=12),
            RelicId::Bonfire => rng.random_range(0..=12),
            RelicId::Temperance => rng.random_range(0..=100),
            RelicId::Kintsugi => rng.random_range(0..=12),
            RelicId::LotusBloom => rng.random_range(0..=16),
            RelicId::WallWeaver => rng.random_range(0..=36),
            RelicId::Heirloom => rng.random_range(0..=12),
            RelicId::Kindling => rng.random_range(0..=30),
            RelicId::HungryGhost => rng.random_range(0..=200),
            _ => 0,
        };
        if value > 0 {
            counters.insert(id, value);
        }
    }
    (relics, debuffed, counters)
}

fn scoring_relic_pool() -> &'static [RelicId] {
    &[
        RelicId::AncestorEcho,
        RelicId::BlueTilesWhiteDragon,
        RelicId::Bonfire,
        RelicId::ChainReaction,
        RelicId::Chastity,
        RelicId::ChowLine,
        RelicId::ClosedGate,
        RelicId::CrownOfPatterns,
        RelicId::CurioCabinet,
        RelicId::DoraCrown,
        RelicId::DragonEcho,
        RelicId::DragonRage,
        RelicId::EdgeRunner,
        RelicId::EasterEgg,
        RelicId::EulersNumber,
        RelicId::EvenKeel,
        RelicId::GardenKeeper,
        RelicId::Geese,
        RelicId::GhostHand,
        RelicId::GlassCannon,
        RelicId::GoldenEngine,
        RelicId::GreenTilesGreenDragon,
        RelicId::Hanami,
        RelicId::Heirloom,
        RelicId::HighTide,
        RelicId::HonorFury,
        RelicId::Humility,
        RelicId::HungryGhost,
        RelicId::Ikebana,
        RelicId::JadeSerpent,
        RelicId::KanDrum,
        RelicId::Kindling,
        RelicId::Kintsugi,
        RelicId::KongsBlessing,
        RelicId::LapisSerpent,
        RelicId::LastBreath,
        RelicId::LotusBloom,
        RelicId::LowTide,
        RelicId::LuckySeven,
        RelicId::MeltingIce,
        RelicId::Minimalist,
        RelicId::MirrorTile,
        RelicId::Momentum,
        RelicId::MonarchButterfly,
        RelicId::MultiplierMaster,
        RelicId::Obsession,
        RelicId::OpenGate,
        RelicId::PairPower,
        RelicId::PaperLantern,
        RelicId::PiConstant,
        RelicId::PlainDealing,
        RelicId::Rakuware,
        RelicId::RedTilesRedDragon,
        RelicId::RiverRunner,
        RelicId::RubySerpent,
        RelicId::SequenceSurge,
        RelicId::ShadowHand,
        RelicId::SilkMoth,
        RelicId::SilkThread,
        RelicId::Snowball,
        RelicId::SolitarySage,
        RelicId::StoneLantern,
        RelicId::StrengthInNumbers,
        RelicId::Taotie,
        RelicId::TeaCeremony,
        RelicId::Temperance,
        RelicId::TilePolisher,
        RelicId::Tourist,
        RelicId::TripletBoost,
        RelicId::TurtleShell,
        RelicId::VoiceOfTheElite,
        RelicId::VoiceOfThePeople,
        RelicId::WallWeaver,
        RelicId::WayOfPairs,
        RelicId::WayOfPurity,
        RelicId::WayOfSequences,
        RelicId::WayOfTriplets,
        RelicId::WhiteDragonsHush,
        RelicId::WindReader,
        RelicId::XxxlEgg,
    ]
}

fn random_rules(include_rules: bool, rng: &mut StdRng) -> Vec<RuleModifier> {
    if !include_rules {
        return Vec::new();
    }
    let candidates = [
        RuleModifier::SequenceWrap,
        RuleModifier::NoSequences,
        RuleModifier::HonorTripleScore,
        RuleModifier::NoSequenceBonus,
        RuleModifier::PairsScoreZero,
        RuleModifier::SequencesHalved,
        RuleModifier::RequireHonor,
        RuleModifier::NoFlowerWildcards,
    ];
    let mut rules = Vec::new();
    for rule in candidates {
        if rng.random_range(0..12) == 0 {
            rules.push(rule);
        }
    }
    if rules.contains(&RuleModifier::NoSequences) {
        rules.retain(|rule| *rule != RuleModifier::SequenceWrap);
    }
    rules
}

fn random_special_structure(rng: &mut StdRng) -> (String, Vec<Tile>, Vec<DetectedMeld>) {
    let builders: &[fn(&mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>)] = &[
        special_tanyao,
        special_toitoi,
        special_yakuhai,
        special_daisangen,
        special_shousangen,
        special_iipeikou,
        special_ryanpeikou,
        special_sanshoku_doujun,
        special_sanshoku_doukou,
        special_ittsu,
        special_honitsu,
        special_chinitsu,
        special_junchan,
        special_chanta,
        special_honroutou,
        special_chiitoitsu,
        special_kokushi_musou,
        special_pinfu,
    ];
    let builder = builders.choose(rng).expect("special builders exist");
    let (name, specs) = builder(rng);
    let (tiles, melds) = meld_specs_to_tiles(specs).expect("special fixture specs are valid");
    (name.to_string(), tiles, melds)
}

fn meld_specs_to_tiles(
    specs: Vec<(MeldKind, Vec<Face>)>,
) -> Option<(Vec<Tile>, Vec<DetectedMeld>)> {
    let mut tiles = Vec::new();
    let mut melds = Vec::new();
    let mut face_counts: BTreeMap<(Suit, u8), usize> = BTreeMap::new();
    let mut next_id = 1u32;
    for (kind, faces) in specs {
        let mut ids = Vec::with_capacity(faces.len());
        for face in faces {
            let count = face_counts.entry((face.suit, face.rank)).or_default();
            if *count >= 4 {
                return None;
            }
            *count += 1;
            let tile = Tile::new(face.suit, face.rank, next_id);
            next_id += 1;
            ids.push(tile.id);
            tiles.push(tile);
        }
        melds.push(DetectedMeld {
            kind,
            tile_ids: ids,
        });
    }
    Some((tiles, melds))
}

fn f(suit: Suit, rank: u8) -> Face {
    Face { suit, rank }
}

fn seq(suit: Suit, start: u8) -> Vec<Face> {
    vec![f(suit, start), f(suit, start + 1), f(suit, start + 2)]
}

fn repeat(kind: MeldKind, suit: Suit, rank: u8, count: usize) -> (MeldKind, Vec<Face>) {
    (kind, vec![f(suit, rank); count])
}

fn pair(suit: Suit, rank: u8) -> (MeldKind, Vec<Face>) {
    repeat(MeldKind::Pair, suit, rank, 2)
}

fn triplet(suit: Suit, rank: u8) -> (MeldKind, Vec<Face>) {
    repeat(MeldKind::Triplet, suit, rank, 3)
}

fn sequence(suit: Suit, start: u8) -> (MeldKind, Vec<Face>) {
    (MeldKind::Sequence, seq(suit, start))
}

fn random_number_suit(rng: &mut StdRng) -> Suit {
    *[Suit::Manzu, Suit::Souzu, Suit::Pinzu]
        .choose(rng)
        .expect("number suits exist")
}

fn other_number_suit(suit: Suit) -> Suit {
    match suit {
        Suit::Manzu => Suit::Souzu,
        Suit::Souzu => Suit::Pinzu,
        Suit::Pinzu => Suit::Manzu,
        _ => Suit::Manzu,
    }
}

fn special_tanyao(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "tanyao",
        vec![
            sequence(suit, 2),
            sequence(suit, 3),
            triplet(other_number_suit(suit), 6),
            triplet(Suit::Pinzu, 8),
            pair(Suit::Souzu, 5),
        ],
    )
}

fn special_toitoi(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "toitoi",
        vec![
            triplet(suit, 2),
            triplet(suit, 5),
            triplet(other_number_suit(suit), 7),
            triplet(Suit::Wind, 1),
            pair(Suit::Dragon, 2),
        ],
    )
}

fn special_yakuhai(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "yakuhai",
        vec![
            triplet(Suit::Dragon, 1),
            sequence(Suit::Manzu, 2),
            sequence(Suit::Souzu, 4),
            triplet(Suit::Pinzu, 8),
            pair(Suit::Wind, 2),
        ],
    )
}

fn special_daisangen(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "daisangen",
        vec![
            triplet(Suit::Dragon, 1),
            triplet(Suit::Dragon, 2),
            triplet(Suit::Dragon, 3),
            sequence(Suit::Manzu, 4),
            pair(Suit::Pinzu, 2),
        ],
    )
}

fn special_shousangen(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "shousangen",
        vec![
            triplet(Suit::Dragon, 1),
            triplet(Suit::Dragon, 2),
            pair(Suit::Dragon, 3),
            sequence(Suit::Manzu, 4),
            sequence(Suit::Souzu, 6),
        ],
    )
}

fn special_iipeikou(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "iipeikou",
        vec![
            sequence(suit, 2),
            sequence(suit, 2),
            sequence(other_number_suit(suit), 4),
            triplet(Suit::Wind, 1),
            pair(Suit::Pinzu, 8),
        ],
    )
}

fn special_ryanpeikou(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "ryanpeikou",
        vec![
            sequence(suit, 1),
            sequence(suit, 1),
            sequence(suit, 5),
            sequence(suit, 5),
            pair(other_number_suit(suit), 4),
        ],
    )
}

fn special_sanshoku_doujun(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "sanshoku_doujun",
        vec![
            sequence(Suit::Manzu, 3),
            sequence(Suit::Souzu, 3),
            sequence(Suit::Pinzu, 3),
            triplet(Suit::Wind, 4),
            pair(Suit::Dragon, 2),
        ],
    )
}

fn special_sanshoku_doukou(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "sanshoku_doukou",
        vec![
            triplet(Suit::Manzu, 5),
            triplet(Suit::Souzu, 5),
            triplet(Suit::Pinzu, 5),
            sequence(Suit::Manzu, 1),
            pair(Suit::Wind, 2),
        ],
    )
}

fn special_ittsu(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "ittsu",
        vec![
            sequence(suit, 1),
            sequence(suit, 4),
            sequence(suit, 7),
            triplet(Suit::Dragon, 2),
            pair(other_number_suit(suit), 5),
        ],
    )
}

fn special_honitsu(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "honitsu",
        vec![
            sequence(suit, 2),
            sequence(suit, 6),
            triplet(suit, 9),
            triplet(Suit::Wind, 3),
            pair(Suit::Dragon, 1),
        ],
    )
}

fn special_chinitsu(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "chinitsu",
        vec![
            sequence(suit, 1),
            sequence(suit, 4),
            sequence(suit, 7),
            triplet(suit, 5),
            pair(suit, 9),
        ],
    )
}

fn special_junchan(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    let other = other_number_suit(suit);
    (
        "junchan",
        vec![
            sequence(suit, 1),
            sequence(suit, 7),
            sequence(other, 1),
            triplet(other, 9),
            pair(Suit::Pinzu, 1),
        ],
    )
}

fn special_chanta(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "chanta",
        vec![
            sequence(suit, 1),
            sequence(other_number_suit(suit), 7),
            triplet(Suit::Wind, 1),
            triplet(Suit::Dragon, 3),
            pair(Suit::Manzu, 9),
        ],
    )
}

fn special_honroutou(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "honroutou",
        vec![
            triplet(Suit::Manzu, 1),
            triplet(Suit::Souzu, 9),
            triplet(Suit::Pinzu, 1),
            triplet(Suit::Wind, 4),
            pair(Suit::Dragon, 2),
        ],
    )
}

fn special_chiitoitsu(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    (
        "chiitoitsu",
        vec![
            pair(Suit::Manzu, 1),
            pair(Suit::Manzu, 4),
            pair(Suit::Souzu, 2),
            pair(Suit::Souzu, 7),
            pair(Suit::Pinzu, 3),
            pair(Suit::Pinzu, 8),
            pair(Suit::Dragon, 1),
        ],
    )
}

fn special_kokushi_musou(_rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let mut specs = Vec::new();
    for face in [
        f(Suit::Manzu, 9),
        f(Suit::Souzu, 1),
        f(Suit::Souzu, 9),
        f(Suit::Pinzu, 1),
        f(Suit::Pinzu, 9),
        f(Suit::Wind, 1),
        f(Suit::Wind, 2),
        f(Suit::Wind, 3),
        f(Suit::Wind, 4),
        f(Suit::Dragon, 1),
        f(Suit::Dragon, 2),
        f(Suit::Dragon, 3),
    ] {
        specs.push((MeldKind::Single, vec![face]));
    }
    specs.push(pair(Suit::Manzu, 1));
    ("kokushi_musou", specs)
}

fn special_pinfu(rng: &mut StdRng) -> (&'static str, Vec<(MeldKind, Vec<Face>)>) {
    let suit = random_number_suit(rng);
    (
        "pinfu",
        vec![
            sequence(suit, 1),
            sequence(suit, 4),
            sequence(other_number_suit(suit), 2),
            sequence(Suit::Pinzu, 5),
            pair(Suit::Souzu, 6),
        ],
    )
}

fn random_meld_structure(
    target_meld_count: usize,
    rules: &[RuleModifier],
    rng: &mut StdRng,
) -> Option<(Vec<Tile>, Vec<DetectedMeld>)> {
    let mut tiles = Vec::new();
    let mut melds = Vec::new();
    let mut face_counts: BTreeMap<(Suit, u8), usize> = BTreeMap::new();
    let mut next_id = 1u32;
    let must_include_honor = rules.contains(&RuleModifier::RequireHonor);

    for meld_index in 0..target_meld_count {
        let kind = random_meld_kind(target_meld_count, meld_index, rules, rng);
        let force_honor = must_include_honor
            && meld_index + 1 == target_meld_count
            && !tiles
                .iter()
                .any(|t: &Tile| matches!(t.suit, Suit::Wind | Suit::Dragon));
        let faces = random_meld_faces(kind, force_honor, rules, &face_counts, rng)?;
        let mut ids = Vec::with_capacity(faces.len());
        for face in faces {
            let count = face_counts.entry((face.suit, face.rank)).or_default();
            if *count >= 4 {
                return None;
            }
            *count += 1;
            let tile = Tile::new(face.suit, face.rank, next_id);
            next_id += 1;
            ids.push(tile.id);
            tiles.push(tile);
        }
        melds.push(DetectedMeld {
            kind,
            tile_ids: ids,
        });
    }
    Some((tiles, melds))
}

fn random_meld_kind(
    target_meld_count: usize,
    meld_index: usize,
    rules: &[RuleModifier],
    rng: &mut StdRng,
) -> MeldKind {
    if target_meld_count == 5 && meld_index + 1 == target_meld_count {
        return MeldKind::Pair;
    }
    let mut kinds = vec![MeldKind::Pair, MeldKind::Triplet, MeldKind::Kong];
    if !rules.contains(&RuleModifier::NoSequences) {
        kinds.push(MeldKind::Sequence);
    }
    *kinds.choose(rng).expect("non-empty meld kind pool")
}

fn random_meld_faces(
    kind: MeldKind,
    force_honor: bool,
    rules: &[RuleModifier],
    face_counts: &BTreeMap<(Suit, u8), usize>,
    rng: &mut StdRng,
) -> Option<Vec<Face>> {
    match kind {
        MeldKind::Pair => {
            let face = random_repeat_face(force_honor, 2, face_counts, rng)?;
            Some(vec![face, face])
        }
        MeldKind::Triplet => {
            let face = random_repeat_face(force_honor, 3, face_counts, rng)?;
            Some(vec![face, face, face])
        }
        MeldKind::Kong => {
            let face = random_repeat_face(force_honor, 4, face_counts, rng)?;
            Some(vec![face, face, face, face])
        }
        MeldKind::Sequence => random_sequence_faces(force_honor, rules, face_counts, rng),
        MeldKind::Single => None,
    }
}

fn random_repeat_face(
    force_honor: bool,
    count: usize,
    face_counts: &BTreeMap<(Suit, u8), usize>,
    rng: &mut StdRng,
) -> Option<Face> {
    let mut candidates = Vec::new();
    for suit in all_suits() {
        if force_honor && !matches!(suit, Suit::Wind | Suit::Dragon) {
            continue;
        }
        for rank in ranks_for_suit(suit) {
            let current = face_counts.get(&(suit, rank)).copied().unwrap_or(0);
            if current + count <= 4 {
                candidates.push(Face { suit, rank });
            }
        }
    }
    candidates.choose(rng).copied()
}

fn random_sequence_faces(
    force_honor: bool,
    rules: &[RuleModifier],
    face_counts: &BTreeMap<(Suit, u8), usize>,
    rng: &mut StdRng,
) -> Option<Vec<Face>> {
    if force_honor {
        return None;
    }
    let mut candidates = Vec::new();
    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
        for start in 1..=7 {
            let ranks = [start, start + 1, start + 2];
            if ranks
                .iter()
                .all(|rank| face_counts.get(&(suit, *rank)).copied().unwrap_or(0) + 1 <= 4)
            {
                candidates.push(ranks.map(|rank| Face { suit, rank }).to_vec());
            }
        }
        if rules.contains(&RuleModifier::SequenceWrap) {
            for ranks in [[8, 9, 1], [9, 1, 2]] {
                if ranks
                    .iter()
                    .all(|rank| face_counts.get(&(suit, *rank)).copied().unwrap_or(0) + 1 <= 4)
                {
                    candidates.push(ranks.map(|rank| Face { suit, rank }).to_vec());
                }
            }
        }
    }
    candidates.choose(rng).cloned()
}

fn all_suits() -> [Suit; 5] {
    [
        Suit::Manzu,
        Suit::Souzu,
        Suit::Pinzu,
        Suit::Wind,
        Suit::Dragon,
    ]
}

fn ranks_for_suit(suit: Suit) -> std::ops::RangeInclusive<u8> {
    match suit {
        Suit::Manzu | Suit::Souzu | Suit::Pinzu => 1..=9,
        Suit::Wind => 1..=4,
        Suit::Dragon => 1..=3,
        Suit::Flower | Suit::Season => 1..=4,
    }
}

fn meld_fixture(tiles: &[Tile], meld: &DetectedMeld) -> MeldFixture {
    MeldFixture {
        kind: meld_kind_name(meld.kind),
        tile_ids: meld.tile_ids.clone(),
        labels: meld
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|tile| tile.id == *id))
            .map(Tile::label)
            .collect(),
    }
}

fn tile_fixture(tile: Tile) -> TileFixture {
    TileFixture {
        id: tile.id,
        suit: suit_name(tile.suit),
        rank: tile.rank,
        label: tile.label(),
    }
}

fn context_fixture(context: &GeneratedContext) -> ContextFixture {
    ContextFixture {
        round_wind: context.round_wind,
        bonus_round_wind: context.bonus_round_wind,
        dora_faces: context
            .dora_faces
            .iter()
            .map(|(suit, rank)| FaceFixture {
                suit: suit_name(*suit),
                rank: *rank,
            })
            .collect(),
        yaku_levels: yaku_levels_fixture(context.yaku_levels.as_ref()),
        relics: context.relics.iter().copied().map(relic_name).collect(),
        debuffed_relics: context
            .debuffed_relics
            .iter()
            .copied()
            .map(relic_name)
            .collect(),
        relic_counters: relic_counters_fixture(&context.relic_counters),
        scored_last_turn: context.scored_last_turn,
        plays_used: context.plays_used,
        is_final_play: context.is_final_play,
        yen: context.yen,
        total_score: context.total_score,
        inject_chicken_if_no_yaku: true,
    }
}

fn yaku_levels_fixture(levels: Option<&YakuLevels>) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    if let Some(levels) = levels {
        for yaku in YakuKind::all() {
            let level = levels.level_of(*yaku);
            if level > 1 {
                out.insert(yaku_name(*yaku), level);
            }
        }
    }
    out
}

fn relic_counters_fixture(counters: &BTreeMap<RelicId, i32>) -> BTreeMap<String, i32> {
    counters
        .iter()
        .map(|(&id, &value)| (relic_name(id), value))
        .collect()
}

fn score_fixture(score: &ScoreBreakdown) -> ScoreFixture {
    ScoreFixture {
        base_fu: score.base_fu,
        base_points: score.base_points,
        final_fu: score.final_fu,
        final_han: score.final_han,
        total: score.total,
        flower_yen: score.flower_yen,
        detected_yaku: score.detected_yaku.iter().copied().map(yaku_name).collect(),
        base_steps: score.base_steps.iter().map(step_fixture).collect(),
        steps: score.steps.iter().map(step_fixture).collect(),
        scored_meld_kinds: score
            .scored_meld_kinds
            .iter()
            .copied()
            .map(meld_kind_name)
            .collect(),
    }
}

fn step_fixture(step: &ScoreStep) -> StepFixture {
    StepFixture {
        source: step.source.clone(),
        kind: step_kind_name(step.kind),
        tile_ids: step.tile_ids.clone(),
        running_fu: step.running_fu,
        running_han: step.running_han,
        running_total: step.running_total,
    }
}

fn suit_name(suit: Suit) -> String {
    match suit {
        Suit::Manzu => "manzu",
        Suit::Souzu => "souzu",
        Suit::Pinzu => "pinzu",
        Suit::Wind => "wind",
        Suit::Dragon => "dragon",
        Suit::Flower => "flower",
        Suit::Season => "season",
    }
    .to_string()
}

fn meld_kind_name(kind: MeldKind) -> String {
    match kind {
        MeldKind::Pair => "pair",
        MeldKind::Triplet => "triplet",
        MeldKind::Sequence => "sequence",
        MeldKind::Kong => "kong",
        MeldKind::Single => "single",
    }
    .to_string()
}

fn step_kind_name(kind: StepKind) -> String {
    match kind {
        StepKind::Fu => "fu",
        StepKind::Han => "han",
        StepKind::Yen => "yen",
        StepKind::Final => "final",
    }
    .to_string()
}

fn yaku_name(yaku: YakuKind) -> String {
    serde_json::to_value(yaku)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{yaku:?}"))
}

fn rule_name(rule: RuleModifier) -> String {
    serde_json::to_value(rule)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| rule.name().to_string())
}

fn relic_name(relic: RelicId) -> String {
    serde_json::to_value(relic)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{relic:?}"))
}
