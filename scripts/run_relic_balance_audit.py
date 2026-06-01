#!/usr/bin/env python3
"""Run a repeatable relic-balance audit and emit machine/human artifacts.

This script automates a formalized balance job:
1) Bot export (observational metrics)
2) Forced relic sweep (causal lift)
3) Tier/price-normalized scoring
4) Markdown summary + CSV detail

Example:
    python3 scripts/run_relic_balance_audit.py --runs-bot 250 --runs-forced 25
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import math
import os
import statistics
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DEFAULT_CONFIG_PATH = Path("scripts/relic_balance_audit_config.json")
DEFAULT_BINARY = Path("target/release/mahjuro")


@dataclass
class PoolSpec:
    name: str
    meta_depth: int | None


@dataclass
class RelicMetricRow:
    pool: str
    name: str
    relic_id: str
    rarity: str
    price: int
    offers: int
    bought: int
    take_rate_pct: float
    win_lift_obs_pct: float
    ante_lift: float
    score_share_pct: float
    forced_win_lift_pct: float | None
    price_efficiency: float | None
    price_efficiency_percentile: float | None
    overtuned_hits: int
    undertuned_hits: int
    status: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG_PATH)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--runs-bot", type=int, default=250)
    parser.add_argument("--runs-forced", type=int, default=25)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("balance-audit"),
        help="Parent folder for run artifacts",
    )
    parser.add_argument(
        "--timestamp",
        default=dt.datetime.now().strftime("%Y%m%d-%H%M%S"),
        help="Run stamp used in output folder name",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build --release before running jobs",
    )
    parser.add_argument(
        "--skip-runs",
        action="store_true",
        help="Only analyze existing JSON artifacts in output folder",
    )
    return parser.parse_args()


def run_cmd(cmd: list[str], cwd: Path) -> None:
    print("$", " ".join(cmd))
    proc = subprocess.run(cmd, cwd=cwd, check=False)
    if proc.returncode != 0:
        raise RuntimeError(f"command failed with exit {proc.returncode}: {' '.join(cmd)}")


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def median_or_zero(xs: list[float]) -> float:
    if not xs:
        return 0.0
    return float(statistics.median(xs))


def percentile_rank(sorted_values: list[float], value: float) -> float:
    if not sorted_values:
        return 0.0
    idx = 0
    for i, v in enumerate(sorted_values):
        if value <= v:
            idx = i
            break
    else:
        idx = len(sorted_values) - 1
    if len(sorted_values) == 1:
        return 1.0
    return idx / (len(sorted_values) - 1)


def safe_div(num: float, den: float) -> float:
    if den == 0:
        return 0.0
    return num / den


def load_config(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"config not found: {path}")
    return load_json(path)


def parse_pools(config: dict[str, Any]) -> list[PoolSpec]:
    pools = []
    for p in config.get("pools", []):
        pools.append(PoolSpec(name=p["name"], meta_depth=p.get("meta_depth")))
    if not pools:
        pools.append(PoolSpec(name="full", meta_depth=None))
    return pools


def index_by_name(rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {str(r["name"]): r for r in rows}


def relic_meta(root: Path) -> dict[str, dict[str, str]]:
    defs = load_json(root / "assets/data/relics.json")
    out: dict[str, dict[str, str]] = {}
    for d in defs:
        out[str(d["name"])] = {
            "id": str(d["id"]),
            "rarity": str(d["rarity"]),
        }
    return out


def build_pool_rows(
    pool: PoolSpec,
    bot_report: dict[str, Any],
    forced_report: dict[str, Any],
    cfg: dict[str, Any],
    meta: dict[str, dict[str, str]],
) -> list[RelicMetricRow]:
    thresholds = cfg["thresholds"]
    price_by_rarity = cfg["price_by_rarity"]
    min_offers = int(thresholds.get("min_offers", 10))
    min_bought_for_win_lift = int(thresholds.get("min_bought_for_win_lift", 20))

    derived = bot_report.get("derived", {})
    aggregate = bot_report.get("aggregate", {})
    sums = aggregate.get("sums", {})
    total_structure_pts = float(sums.get("total_structure_trigger_points", 0))

    bought = index_by_name(derived.get("relics_bought", []))
    funnel = index_by_name(derived.get("relics_shop_funnel", []))
    depth = index_by_name(derived.get("relics_depth_split", []))
    attrib = index_by_name(derived.get("relics_score_attribution", []))

    forced_baseline = forced_report.get("baseline_aggregate", {})
    forced_base_win = 100.0 * safe_div(
        float(forced_baseline.get("victories", 0)),
        float(forced_baseline.get("runs", 1)),
    )
    forced_rows = forced_report.get("relics", [])
    forced_win_by_name: dict[str, float] = {}
    for r in forced_rows:
        agg = r.get("aggregate", {})
        win = 100.0 * safe_div(float(agg.get("victories", 0)), float(agg.get("runs", 1)))
        forced_win_by_name[str(r.get("relic", ""))] = win

    names = sorted(set(funnel) | set(bought) | set(depth) | set(attrib), key=str.casefold)
    rows: list[RelicMetricRow] = []
    for name in names:
        f = funnel.get(name, {})
        offers = int(f.get("offers", 0))
        if offers < min_offers:
            continue
        b = bought.get(name, {})
        d = depth.get(name, {})
        a = attrib.get(name, {})

        rarity = str((meta.get(name) or {}).get("rarity", f.get("rarity", "common"))).lower()
        rid = str((meta.get(name) or {}).get("id", ""))
        price = int(price_by_rarity.get(rarity, 6))
        bought_n = int(b.get("bought", f.get("bought", 0)))
        take = float(f.get("take_rate_pct", 0.0))
        win_obs = float(b.get("delta_vs_baseline_pct", 0.0))
        ante = float(d.get("delta_antes", 0.0))
        score_pts = float(a.get("score_pts", 0.0))
        score_share = 100.0 * safe_div(score_pts, total_structure_pts)

        forced_win = forced_win_by_name.get(name)
        forced_lift = None if forced_win is None else forced_win - forced_base_win
        eff = None if forced_lift is None else forced_lift / max(price, 1)

        if bought_n < min_bought_for_win_lift:
            win_obs_for_flags = 0.0
        else:
            win_obs_for_flags = win_obs

        rows.append(
            RelicMetricRow(
                pool=pool.name,
                name=name,
                relic_id=rid,
                rarity=rarity,
                price=price,
                offers=offers,
                bought=bought_n,
                take_rate_pct=take,
                win_lift_obs_pct=win_obs,
                ante_lift=ante,
                score_share_pct=score_share,
                forced_win_lift_pct=forced_lift,
                price_efficiency=eff,
                price_efficiency_percentile=None,
                overtuned_hits=0,
                undertuned_hits=0,
                status="ok",
            )
        )

    # Tier-normalized efficiency percentile
    by_rarity_values: dict[str, list[float]] = {}
    for r in rows:
        if r.price_efficiency is not None:
            by_rarity_values.setdefault(r.rarity, []).append(r.price_efficiency)
    for rarity, vals in by_rarity_values.items():
        vals.sort()
        by_rarity_values[rarity] = vals

    over = thresholds["overtuned"]
    under = thresholds["undertuned"]
    for r in rows:
        if r.price_efficiency is not None:
            vals = by_rarity_values.get(r.rarity, [])
            r.price_efficiency_percentile = percentile_rank(vals, r.price_efficiency)

        over_hits = 0
        if r.take_rate_pct >= float(over["take_rate_pct"]):
            over_hits += 1
        if (r.forced_win_lift_pct or 0.0) >= float(over["forced_win_lift_pct"]):
            over_hits += 1
        if r.ante_lift >= float(over["ante_lift"]):
            over_hits += 1
        if r.score_share_pct >= float(over["score_share_pct"]):
            over_hits += 1
        if (r.price_efficiency_percentile or 0.0) >= float(over["price_efficiency_percentile"]):
            over_hits += 1

        under_hits = 0
        if r.take_rate_pct <= float(under["take_rate_pct"]):
            under_hits += 1
        if (r.forced_win_lift_pct or 0.0) <= float(under["forced_win_lift_pct"]):
            under_hits += 1
        if r.ante_lift <= float(under["ante_lift"]):
            under_hits += 1
        if r.score_share_pct <= float(under["score_share_pct"]):
            under_hits += 1
        if (r.price_efficiency_percentile or 1.0) <= float(under["price_efficiency_percentile"]):
            under_hits += 1

        r.overtuned_hits = over_hits
        r.undertuned_hits = under_hits
        if over_hits >= int(over["min_hits"]):
            r.status = "Overtuned"
        elif under_hits >= int(under["min_hits"]):
            r.status = "Undertuned"
        else:
            r.status = "OK"

    return rows


def write_csv(path: Path, rows: list[RelicMetricRow]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(
            [
                "pool",
                "name",
                "relic_id",
                "rarity",
                "price",
                "offers",
                "bought",
                "take_rate_pct",
                "win_lift_obs_pct",
                "ante_lift",
                "score_share_pct",
                "forced_win_lift_pct",
                "price_efficiency",
                "price_efficiency_percentile",
                "overtuned_hits",
                "undertuned_hits",
                "status",
            ]
        )
        for r in rows:
            w.writerow(
                [
                    r.pool,
                    r.name,
                    r.relic_id,
                    r.rarity,
                    r.price,
                    r.offers,
                    r.bought,
                    f"{r.take_rate_pct:.3f}",
                    f"{r.win_lift_obs_pct:.3f}",
                    f"{r.ante_lift:.3f}",
                    f"{r.score_share_pct:.3f}",
                    "" if r.forced_win_lift_pct is None else f"{r.forced_win_lift_pct:.3f}",
                    "" if r.price_efficiency is None else f"{r.price_efficiency:.4f}",
                    ""
                    if r.price_efficiency_percentile is None
                    else f"{r.price_efficiency_percentile:.4f}",
                    r.overtuned_hits,
                    r.undertuned_hits,
                    r.status,
                ]
            )


def top_rows(rows: list[RelicMetricRow], status: str, n: int = 8) -> list[RelicMetricRow]:
    filt = [r for r in rows if r.status == status]
    if status == "Overtuned":
        filt.sort(
            key=lambda r: (
                r.overtuned_hits,
                r.forced_win_lift_pct or -math.inf,
                r.take_rate_pct,
                r.score_share_pct,
            ),
            reverse=True,
        )
    else:
        filt.sort(
            key=lambda r: (
                r.undertuned_hits,
                -(r.forced_win_lift_pct or math.inf),
                -r.take_rate_pct,
            ),
            reverse=True,
        )
    return filt[:n]


def tier_snapshot(rows: list[RelicMetricRow]) -> dict[str, dict[str, float]]:
    out: dict[str, dict[str, float]] = {}
    for rarity in sorted({r.rarity for r in rows}):
        rs = [r for r in rows if r.rarity == rarity]
        out[rarity] = {
            "n": float(len(rs)),
            "take_median": median_or_zero([r.take_rate_pct for r in rs]),
            "forced_median": median_or_zero(
                [r.forced_win_lift_pct for r in rs if r.forced_win_lift_pct is not None]
            ),
            "eff_median": median_or_zero(
                [r.price_efficiency for r in rs if r.price_efficiency is not None]
            ),
        }
    return out


def write_markdown_summary(
    path: Path,
    rows_by_pool: dict[str, list[RelicMetricRow]],
    run_dir: Path,
    args: argparse.Namespace,
    pools: list[PoolSpec],
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines: list[str] = []
    lines.append("# Relic Balance Audit Summary")
    lines.append("")
    lines.append(f"- Generated: `{dt.datetime.now().isoformat(timespec='seconds')}`")
    lines.append(f"- Output dir: `{run_dir}`")
    lines.append(f"- Bot runs per pool: `{args.runs_bot}`")
    lines.append(f"- Forced runs per relic: `{args.runs_forced}`")
    lines.append("")
    lines.append("## Pools")
    for p in pools:
        lines.append(f"- `{p.name}` (`meta_depth={p.meta_depth}`)")
    lines.append("")

    for pool_name, rows in rows_by_pool.items():
        lines.append(f"## Pool: `{pool_name}`")
        lines.append("")
        tier = tier_snapshot(rows)
        lines.append("### Tier Medians")
        lines.append("| Rarity | n | median take% | median forced Δwin | median price efficiency |")
        lines.append("|---|---:|---:|---:|---:|")
        for rarity in ["common", "uncommon", "rare", "legendary"]:
            t = tier.get(rarity)
            if not t:
                continue
            lines.append(
                f"| {rarity} | {int(t['n'])} | {t['take_median']:.1f} | {t['forced_median']:.2f} | {t['eff_median']:.3f} |"
            )
        lines.append("")

        over = top_rows(rows, "Overtuned")
        under = top_rows(rows, "Undertuned")
        lines.append("### Top Overtuned Candidates")
        if over:
            lines.append("| Relic | Rarity | Price | take% | obs Δwin | forced Δwin | ante Δ | score% | hits |")
            lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
            for r in over:
                lines.append(
                    f"| {r.name} | {r.rarity} | {r.price} | {r.take_rate_pct:.1f} | {r.win_lift_obs_pct:+.1f} | {(r.forced_win_lift_pct or 0.0):+.1f} | {r.ante_lift:+.2f} | {r.score_share_pct:.1f} | {r.overtuned_hits} |"
                )
        else:
            lines.append("_None flagged._")
        lines.append("")

        lines.append("### Top Undertuned Candidates")
        if under:
            lines.append("| Relic | Rarity | Price | take% | obs Δwin | forced Δwin | ante Δ | score% | hits |")
            lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
            for r in under:
                lines.append(
                    f"| {r.name} | {r.rarity} | {r.price} | {r.take_rate_pct:.1f} | {r.win_lift_obs_pct:+.1f} | {(r.forced_win_lift_pct or 0.0):+.1f} | {r.ante_lift:+.2f} | {r.score_share_pct:.1f} | {r.undertuned_hits} |"
                )
        else:
            lines.append("_None flagged._")
        lines.append("")

    lines.append("## Artifacts")
    lines.append(f"- `relic_balance_metrics.csv`")
    for p in pools:
        lines.append(f"- `{p.name}-bot.json`")
        lines.append(f"- `{p.name}-forced.json`")
    lines.append("")

    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)

    cfg = load_config(args.config)
    pools = parse_pools(cfg)

    run_dir = args.output_dir / args.timestamp
    run_dir.mkdir(parents=True, exist_ok=True)

    if not args.skip_build:
        run_cmd(["cargo", "build", "--release"], cwd=root)

    if not args.skip_runs:
        for pool in pools:
            bot_out = run_dir / f"{pool.name}-bot.json"
            forced_out = run_dir / f"{pool.name}-forced.json"

            bot_cmd = [
                str(args.binary),
                "bot",
                str(args.runs_bot),
            ]
            if pool.meta_depth is not None:
                bot_cmd += ["--meta-depth", str(pool.meta_depth)]
            bot_cmd += ["-o", str(bot_out), "--output-format", "json"]
            run_cmd(bot_cmd, cwd=root)

            forced_cmd = [
                str(args.binary),
                "forced-relic-sweep",
                "--runs",
                str(args.runs_forced),
            ]
            if pool.meta_depth is not None:
                forced_cmd += ["--meta-depth", str(pool.meta_depth)]
            forced_cmd += ["--export-json", str(forced_out)]
            run_cmd(forced_cmd, cwd=root)

    # Analyze artifacts
    meta = relic_meta(root)
    all_rows: list[RelicMetricRow] = []
    rows_by_pool: dict[str, list[RelicMetricRow]] = {}
    for pool in pools:
        bot_path = run_dir / f"{pool.name}-bot.json"
        forced_path = run_dir / f"{pool.name}-forced.json"
        if not bot_path.exists() or not forced_path.exists():
            raise FileNotFoundError(
                f"missing artifact for pool {pool.name}: {bot_path} / {forced_path}"
            )
        bot_report = load_json(bot_path)
        forced_report = load_json(forced_path)
        rows = build_pool_rows(pool, bot_report, forced_report, cfg, meta)
        rows_by_pool[pool.name] = rows
        all_rows.extend(rows)

    write_csv(run_dir / "relic_balance_metrics.csv", all_rows)
    write_markdown_summary(
        run_dir / "relic-balance-summary.md",
        rows_by_pool,
        run_dir,
        args,
        pools,
    )

    print(f"\nAudit complete: {run_dir}")
    print(f"- {run_dir / 'relic_balance_metrics.csv'}")
    print(f"- {run_dir / 'relic-balance-summary.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
