//! Criterion benchmarks for headless bot hot paths, split so costs are visible:
//! end-to-end `pick_best_play`, mask enumeration, validate/structure filtering, and
//! scoring over precomputed masks.
//!
//! Run (release recommended):
//!   cargo bench --bench bot
//!
//! A short dashboard prints first (mask counts and how many reach scoring). Throughput
//! on substeps is **per candidate mask** so you can compare steps fairly.

use criterion::{Criterion, Throughput, black_box};
use mahjuro::bot::{
    bench_count_masks_positive_score, bench_count_masks_validate_structure,
    bench_enumerate_play_masks, bench_evaluate_play_masks, bench_fixture_run, pick_best_play,
};
use mahjuro::core::tile::{Suit, Tile};
use mahjuro::game::run::RunState;

fn t(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

/// Build once per process: `RunState::new_demo()` is not stable across calls, so the dashboard
/// and timed loops must share the same [`RunState`] instances.
fn build_fixtures() -> Vec<(&'static str, RunState)> {
    vec![
        ("demo_default", RunState::new_demo()),
        (
            "flowers_11",
            bench_fixture_run(vec![
                t(Suit::Characters, 2, 1),
                t(Suit::Characters, 3, 2),
                t(Suit::Characters, 5, 3),
                t(Suit::Characters, 5, 4),
                t(Suit::Characters, 5, 5),
                t(Suit::Bamboos, 7, 6),
                t(Suit::Bamboos, 8, 7),
                t(Suit::Dragon, 1, 8),
                t(Suit::Dragon, 1, 9),
                t(Suit::Flower, 1, 10),
                t(Suit::Flower, 2, 11),
            ]),
        ),
        (
            "four_flowers_12",
            bench_fixture_run(vec![
                t(Suit::Characters, 1, 1),
                t(Suit::Characters, 2, 2),
                t(Suit::Characters, 3, 3),
                t(Suit::Characters, 7, 4),
                t(Suit::Characters, 7, 5),
                t(Suit::Characters, 7, 6),
                t(Suit::Wind, 1, 7),
                t(Suit::Wind, 1, 8),
                t(Suit::Flower, 1, 9),
                t(Suit::Flower, 2, 10),
                t(Suit::Flower, 3, 11),
                t(Suit::Flower, 4, 12),
            ]),
        ),
    ]
}

fn print_dashboard(fixtures: &[(&'static str, RunState)]) {
    eprintln!("\n=== bot bench — candidate pipeline (one-time) ===");
    eprintln!(
        "{:<16} {:>4} {:>10} {:>12} {:>12}",
        "fixture", "hand", "masks", "val+struct", "pos_score"
    );
    for (slug, run) in fixtures {
        let hand = run.hand();
        let commit_rules = run.validation_rules_for_structure_commits();
        let masks = bench_enumerate_play_masks(hand, &commit_rules);
        let n = masks.len();
        let v = bench_count_masks_validate_structure(&run, hand, &masks);
        let p = bench_count_masks_positive_score(&run, hand, &masks);
        eprintln!(
            "{:<16} {:>4} {:>10} {:>12} {:>12}",
            slug,
            hand.len(),
            n,
            v,
            p
        );
    }
    eprintln!();
}

fn register_fixture(c: &mut Criterion, slug: &str, run: RunState) {
    let commit_rules = run.validation_rules_for_structure_commits();
    let hand = run.hand();
    let masks = bench_enumerate_play_masks(hand, &commit_rules);
    let n = masks.len() as u64;
    let tp = Throughput::Elements(n.max(1));

    let mut group = c.benchmark_group(format!("pick_best/{slug}"));
    group.sample_size(50);

    group.bench_function(format!("00_end_to_end (masks={n})"), |b| {
        b.iter(|| pick_best_play(black_box(&run)))
    });

    group.throughput(tp);
    group.bench_function(format!("01_enumerate_masks (out={n})"), |b| {
        b.iter(|| {
            bench_enumerate_play_masks(black_box(hand), black_box(commit_rules.as_slice()));
        })
    });

    group.bench_function(format!("02_validate_and_structure (scan {n})"), |b| {
        b.iter(|| {
            bench_count_masks_validate_structure(
                black_box(&run),
                black_box(hand),
                black_box(masks.as_slice()),
            )
        })
    });

    group.bench_function(format!("03_evaluate_precomputed (scan {n})"), |b| {
        b.iter(|| {
            bench_evaluate_play_masks(
                black_box(&run),
                black_box(hand),
                black_box(masks.as_slice()),
            )
        })
    });

    group.bench_function(format!("04_count_positive_score (scan {n})"), |b| {
        b.iter(|| {
            bench_count_masks_positive_score(
                black_box(&run),
                black_box(hand),
                black_box(masks.as_slice()),
            )
        })
    });

    group.finish();
}

fn main() {
    let fixtures = build_fixtures();
    print_dashboard(&fixtures);
    let mut criterion = Criterion::default().configure_from_args();
    for (slug, run) in fixtures {
        register_fixture(&mut criterion, slug, run);
    }
    criterion.final_summary();
}
