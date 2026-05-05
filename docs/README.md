# Bot Balance Assets

This folder stores the current balance-bot dataset and the charts generated from it:

- `bot_balance_runs.json`
- `bot_balance_summary.png`
- `bot_balance_deaths_by_ante.png`
- `bot_balance_economy.png`
- `bot_balance_survival_heatmap.png`
- `bot_balance_snapshot_tradeoffs.png`

## Refreshing The Graphs

Use the `bot-graph` subcommand from the repo root:

```bash
cargo run --release -- bot-graph 10000 --slug baseline_10k --label "Baseline\n(10k runs)"
```

That command:

1. runs the headless bot
2. builds a snapshot from the aggregate stats
3. inserts or replaces the snapshot in `docs/bot_balance_runs.json`
4. regenerates the chart PNGs in `docs/`

If `--slug` is omitted, the app derives one from the active bot config. If `--label` is omitted, the app generates a chart label from the same config.

## Notes

- Always use `--release`.
- Graph generation still shells out to `python3 tools/plot_bot_balance.py`, so local `python3` is required for the PNG step.
- Reusing a slug updates an existing snapshot in place.
- A new slug appends a new snapshot to the dataset.

## Reading The Charts

The bot has much better information than a human player, so these charts are best treated as an upper-bound tuning signal.

### Summary

`bot_balance_summary.png` shows:

- win rate
- average antes cleared
- average total score
- plays and discards used
- blinds cleared and skipped
- relic pressure / gold spent
- earned gold vs final gold

### Deaths By Ante

`bot_balance_deaths_by_ante.png` shows where runs fail.

- leftward spikes mean the wall got earlier
- rightward movement usually means survivability improved
- sharp spikes suggest one concentrated balance cliff

### Economy

`bot_balance_economy.png` compares:

- payout composition
- gold earned
- final gold
- interest / unused-play pressure

### Survival Heatmap

`bot_balance_survival_heatmap.png` shows the percentage of runs that die on each ante for every snapshot.

- it is easier to compare cliffs across many snapshots at once
- darker cells mean more runs died there

### Snapshot Tradeoffs

`bot_balance_snapshot_tradeoffs.png` compares win rate against average score.

- point color tracks average antes cleared
- point size tracks average final gold
- useful for spotting configs that only inflate score or only inflate survival
