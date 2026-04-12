# Bot Balance Graphs

This folder contains the bot-balance snapshot dataset and the generated graphs:

- `bot_balance_runs.json`
- `bot_balance_summary.png`
- `bot_balance_deaths_by_ante.png`
- `bot_balance_economy.png`

## One-command update

Use the `bot-graph` subcommand to run the bot, update `docs/bot_balance_runs.json`, and regenerate the graph PNGs in one step.

Example:

```bash
cargo run --release -- bot-graph 10000 --slug baseline_10k --label "Baseline\n(10k runs)"
```

What it does:

1. Runs `10,000` headless bot games.
2. Computes the graph snapshot fields automatically from the aggregate stats.
3. Inserts or replaces the snapshot in `docs/bot_balance_runs.json` by `slug`.
4. Regenerates the PNGs in `docs/`.

If `--slug` is omitted, the command generates one from the bot config. If `--label` is omitted, it generates a chart label from the mode fields and run count.

## Common examples

Baseline:

```bash
cargo run --release -- bot-graph 10000 --slug baseline_10k --label "Baseline\n(10k runs)"
```

Try a tuning change:

```bash
cargo run --release -- bot-graph 10000 \
  --slug plays5_scale14_10k \
  --label "Plays 5\nScale 1.40\n(10k runs)" \
  --plays 5 \
  --target-scale 1.4
```

Override other mode fields:

```bash
cargo run --release -- bot-graph 10000 \
  --base-target 250 \
  --target-scale 1.3 \
  --plays 5 \
  --discards 3 \
  --gold 4
```

## Notes

- Always use `--release` for bot runs.
- `bot-graph` currently regenerates the PNGs by calling `python3 tools/plot_bot_balance.py`, so `python3` still needs to be available locally.
- Reusing a `slug` updates the existing snapshot in place.
- Using a new `slug` appends a new snapshot to the dataset.

## How to interpret the graphs

The bot is an oracle with perfect information about future tile draws when it evaluates discard lines. Treat these graphs as a ceiling on player power, not as a typical-player baseline.

## Summary graph

`bot_balance_summary.png` has four panels:

- Difficulty: `Win rate %` and `Avg antes`
- Average Total Score (Millions)
- Action Economy: `Avg plays used` and `Avg discards used`
- Blind Pace: `Avg blinds cleared` and `Avg blinds skipped`

How to read it:

- `Avg antes` is the best single difficulty signal. Higher means the bot survives deeper into the run.
- `Win rate %` matters most once snapshots are close to a viable endgame. Before that, `avg_antes` is usually the better comparison.
- `Avg total score` is helpful, but it can be inflated by easier economy or late-run overscore. Do not read it alone.
- `Avg plays used` and `avg discards used` show pressure. If these rise while survivability falls, the run is getting tighter.
- `Avg blinds skipped` is a pacing signal. Higher values usually mean the bot is comfortably clearing targets early enough to bank gold.

## Deaths-by-ante graph

`bot_balance_deaths_by_ante.png` shows where runs fail.

How to read it:

- A spike moving left means the wall got earlier.
- A spike moving right means the run got easier or the economy got stronger.
- A sharp spike means there is a concentrated wall.
- A flatter curve means failure is spread across more of the run.

Rule of thumb:

- If deaths pile up at Ante 2 or 3, the opening is too punishing.
- If deaths mostly pile up at Ante 7 or 8, the run is at least reaching late game consistently.

## Economy graph

`bot_balance_economy.png` compares payout composition and gold outcomes.

How to read it:

- If `Interest` dominates the clear-payout stack, passive hoarding may be too strong.
- If `Unused plays` grows while difficulty drops, the target curve may be too soft.
- `Avg gold earned` shows how much economy the run generated.
- `Avg final gold` shows how much remained unspent; if it stays high, the shop may be too weak or too expensive.
