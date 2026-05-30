# Zodiac balance telemetry and alarms

Use headless bot exports as the primary balance check for zodiac ribbons.

## Recommended run

```bash
cargo run -- bot 2000 --output-file out/bot/zodiac-balance.html --output-runs out/bot/zodiac-runs.jsonl
```

For pure data workflows:

```bash
cargo run -- bot 2000 --output-file out/bot/zodiac-balance.json --output-format json --output-runs out/bot/zodiac-runs.jsonl
```

## Where the signals live

In export `derived`:

- `zodiac_balance[]`: table per zodiac with:
  - acquisition count/rate,
  - use count/rate,
  - `use_per_acquire`,
  - win rate for runs that acquired that zodiac (with Wilson 95% CI),
  - delta vs batch baseline win rate,
  - average final level of the zodiac's primary yaku on runs that acquired it.
- `zodiac_balance_alarms[]`: threshold-driven alarm rows.

## Alarm thresholds

Current defaults:

- **Minimum samples for alarms:** `acquired >= 25`
- **High win delta alarm:** `delta_vs_baseline >= +5.0` and CI lower bound at least `+2.5` above baseline
- **Low win delta alarm:** `delta_vs_baseline <= -5.0` and CI upper bound at least `-2.5` below baseline
- **Low use-per-acquire alarm:** `use_per_acquire <= 0.55`
- **High final-level alarm:** `avg_primary_yaku_level_when_acquired >= 4.0`

## Interpretation checklist

- **Likely overtuned:** repeated `high-win-delta` alarms with stable CI separation.
- **Likely undertuned:** repeated `low-win-delta` alarms with stable CI separation.
- **Likely awkward to use:** repeated `low-use-per-acquire` alarms.
- **Likely runaway scaling:** repeated `high-final-level` alarms.

One batch can be noisy; compare at least 2-3 batches with similar run count and season.
