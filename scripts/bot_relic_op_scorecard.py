#!/usr/bin/env python3
"""Rank relics from a `mahjuro bot -o report.json` export for over/under-use.

Usage:
    python3 scripts/bot_relic_op_scorecard.py bot-report.json
    python3 scripts/bot_relic_op_scorecard.py bot-report.json --top 15 --min-offers 10

Flags relics that are heavily picked *and* correlate with wins / depth / score share.
This is observational (shop picks are biased); pair with `forced-relic-sweep` for causal lift.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class RelicRow:
    name: str
    rarity: str
    take_rate_pct: float
    win_lift: float
    ante_lift: float
    score_share_pct: float
    bought: int
    offers: int
    op_flags: int
    review: str


def load_report(path: Path) -> dict:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def index_by_name(rows: list[dict], key: str = "name") -> dict[str, dict]:
    return {row[key]: row for row in rows}


def build_rows(report: dict, min_offers: int) -> list[RelicRow]:
    derived = report.get("derived") or {}
    aggregate = report.get("aggregate") or {}
    sums = aggregate.get("sums") or {}

    baseline_win = float((derived.get("per_run") or {}).get("win_rate_pct") or 0.0)
    total_score_pts = float(sums.get("total_structure_trigger_points") or 0)

    bought = index_by_name(derived.get("relics_bought") or [])
    funnel = index_by_name(derived.get("relics_shop_funnel") or [])
    depth = index_by_name(derived.get("relics_depth_split") or [])
    attrib = index_by_name(derived.get("relics_score_attribution") or [])

    names = sorted(
        set(bought) | set(funnel) | set(depth) | set(attrib),
        key=str.casefold,
    )

    out: list[RelicRow] = []
    for name in names:
        f = funnel.get(name, {})
        offers = int(f.get("offers") or 0)
        if offers < min_offers:
            continue

        b = bought.get(name, {})
        d = depth.get(name, {})
        a = attrib.get(name, {})

        take_rate = float(f.get("take_rate_pct") or 0.0)
        win_lift = float(b.get("delta_vs_baseline_pct") or 0.0)
        ante_lift = float(d.get("delta_antes") or 0.0)
        score_pts = float(a.get("score_pts") or 0)
        score_share = (100.0 * score_pts / total_score_pts) if total_score_pts > 0 else 0.0
        bought_n = int(b.get("bought") or f.get("bought") or 0)
        rarity = str(b.get("rarity") or f.get("rarity") or "—")

        flags = 0
        if take_rate >= 70.0:
            flags += 1
        if win_lift >= 10.0 and bought_n >= 20:
            flags += 1
        if ante_lift >= 1.0:
            flags += 1
        if score_share >= 5.0:
            flags += 1

        under_flags = 0
        if take_rate <= 20.0 and offers >= min_offers:
            under_flags += 1
        if win_lift <= 0.0 and bought_n >= 20:
            under_flags += 1

        if flags >= 3:
            review = "NERF?"
        elif flags >= 2:
            review = "review"
        elif under_flags >= 2:
            review = "BUFF?"
        elif under_flags >= 1 and take_rate <= 10.0:
            review = "ignored"
        else:
            review = "ok"

        out.append(
            RelicRow(
                name=name,
                rarity=rarity,
                take_rate_pct=take_rate,
                win_lift=win_lift,
                ante_lift=ante_lift,
                score_share_pct=score_share,
                bought=bought_n,
                offers=offers,
                op_flags=flags,
                review=review,
            )
        )

    out.sort(
        key=lambda r: (r.op_flags, r.take_rate_pct, r.win_lift, r.score_share_pct),
        reverse=True,
    )
    return out


def print_table(rows: list[RelicRow], top: int, baseline_win: float) -> None:
    print(f"Baseline win rate: {baseline_win:.1f}%")
    print(
        "Flags: take≥70% | win Δ≥+10 (n≥20) | ante Δ≥+1 | score share≥5%"
    )
    print()
    header = (
        f"{'review':<8} {'relic':<22} {'rarity':<10} {'take%':>6} {'win Δ':>7} "
        f"{'ante Δ':>7} {'score%':>7} {'buy':>5} {'offers':>6} {'flags':>5}"
    )
    print(header)
    print("-" * len(header))
    for row in rows[:top]:
        print(
            f"{row.review:<8} {row.name:<22} {row.rarity:<10} "
            f"{row.take_rate_pct:>5.1f}% {row.win_lift:>+6.1f} {row.ante_lift:>+7.2f} "
            f"{row.score_share_pct:>6.1f}% {row.bought:>5} {row.offers:>6} {row.op_flags:>5}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report_json", type=Path, help="Path to mahjuro bot JSON export")
    parser.add_argument(
        "--top",
        type=int,
        default=20,
        help="Max rows to print (default 20)",
    )
    parser.add_argument(
        "--min-offers",
        type=int,
        default=10,
        help="Min shop offers before including a relic (default 10)",
    )
    args = parser.parse_args()

    if not args.report_json.is_file():
        print(f"error: file not found: {args.report_json}", file=sys.stderr)
        return 1

    report = load_report(args.report_json)
    baseline = float((report.get("derived") or {}).get("per_run", {}).get("win_rate_pct") or 0)
    rows = build_rows(report, args.min_offers)
    if not rows:
        print("No relics matched filters.", file=sys.stderr)
        return 1

    print_table(rows, args.top, baseline)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
