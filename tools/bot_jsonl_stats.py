#!/usr/bin/env python3
"""
Analyze `mahjuro bot --output-runs runs.jsonl` for balance tuning.

Usage:
  python3 tools/bot_jsonl_stats.py path/to/runs.jsonl
  python3 tools/bot_jsonl_stats.py runs.jsonl --bootstrap 2000
  python3 tools/bot_jsonl_stats.py runs.jsonl --fdr --relic-min-buys 20
  python3 tools/bot_jsonl_stats.py runs.jsonl --stratify-relics --ante-bins 2,4

No third-party packages required (stdlib only). Relic associations are not causal.
"""

from __future__ import annotations

import argparse
import json
import math
import random
import sys
from collections import defaultdict
from typing import Any


def wilson_95_pct(k: int, n: int) -> tuple[float, float] | None:
    if n <= 0:
        return None
    k = min(k, n)
    z = 1.96
    nf = float(n)
    p = k / nf
    z2 = z * z
    denom = 1.0 + z2 / nf
    center = (p + z2 / (2.0 * nf)) / denom
    inner = p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)
    half = z * math.sqrt(max(0.0, inner)) / denom
    lo = max(0.0, min(1.0, center - half)) * 100.0
    hi = max(0.0, min(1.0, center + half)) * 100.0
    return lo, hi


def normal_cdf(x: float) -> float:
    return 0.5 * (1.0 + math.erf(x / math.sqrt(2.0)))


def two_proportion_z_p_value(x1: int, n1: int, x0: int, n0: int) -> float:
    """Two-sided p-value for H0: p1 = p0 (normal approximation)."""
    if n1 <= 0 or n0 <= 0:
        return 1.0
    p1, p0 = x1 / n1, x0 / n0
    p_pool = (x1 + x0) / (n1 + n0)
    if p_pool <= 0 or p_pool >= 1:
        return 1.0
    se = math.sqrt(p_pool * (1.0 - p_pool) * (1.0 / n1 + 1.0 / n0))
    if se <= 1e-15:
        return 1.0
    z = (p1 - p0) / se
    p = 2.0 * (1.0 - normal_cdf(abs(z)))
    return max(0.0, min(1.0, p))


def benjamini_hochberg_fdr(names_ps: list[tuple[str, float]], alpha: float = 0.05) -> dict[str, bool]:
    """Benjamini–Hochberg: reject hypotheses with p <= p_cut where p_cut is the k-th smallest p."""
    m = len(names_ps)
    if m == 0:
        return {}
    sorted_p = sorted(p for _, p in names_ps)
    k = 0
    for i, p in enumerate(sorted_p, start=1):
        if p <= (i / m) * alpha:
            k = i
    if k == 0:
        return {n: False for n, _ in names_ps}
    p_cut = sorted_p[k - 1]
    return {n: (p <= p_cut) for n, p in names_ps}


def load_runs(path: str) -> list[dict[str, Any]]:
    runs: list[dict[str, Any]] = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            runs.append(json.loads(line))
    return runs


def death_hazard_table(runs: list[dict[str, Any]]) -> None:
    n = len(runs)
    if n == 0:
        print("No runs.")
        return
    deaths_by_ante: dict[int, int] = defaultdict(int)
    for r in runs:
        if r.get("victory"):
            continue
        a = int(r.get("died_on_ante", 1))
        deaths_by_ante[a] += 1
    if not deaths_by_ante:
        print("\nNo deaths (all wins) — hazard table skipped.")
        return
    max_a = max(deaths_by_ante)
    remaining = n
    print("\nDeath hazard P(die on ante | reached ante), Wilson 95% CI:")
    print(f"{'ante':>5} {'reached':>8} {'deaths':>8} {'hazard%':>9} {'95% CI':>18}")
    for a in range(1, max_a + 1):
        if remaining <= 0:
            break
        dcount = deaths_by_ante.get(a, 0)
        haz = 100.0 * dcount / remaining
        ci = wilson_95_pct(dcount, remaining)
        ci_s = f"{ci[0]:.1f}–{ci[1]:.1f}%" if ci else "—"
        print(f"{a:>5} {remaining:>8} {dcount:>8} {haz:>8.1f}% {ci_s:>18}")
        remaining -= dcount


def bootstrap_means(
    runs: list[dict[str, Any]], iterations: int, seed: int | None
) -> None:
    if not runs:
        return
    rng = random.Random(seed)
    n = len(runs)

    def sample_stat(fn):
        vals = []
        for _ in range(iterations):
            idx = [rng.randrange(n) for _ in range(n)]
            vals.append(fn([runs[i] for i in idx]))
        vals.sort()
        lo = vals[int(0.025 * iterations)]
        hi = vals[int(0.975 * iterations)]
        return lo, hi

    def mean_antes(rs):
        return sum(float(r.get("antes_cleared", 0)) for r in rs) / len(rs)

    def win_rate(rs):
        return 100.0 * sum(1 for r in rs if r.get("victory")) / len(rs)

    ma = sample_stat(mean_antes)
    wr = sample_stat(win_rate)
    print(f"\nBootstrap {iterations} (pctile 2.5–97.5), nonparametric:")
    print(f"  mean antes_cleared: {mean_antes(runs):.4f}  CI [{ma[0]:.4f}, {ma[1]:.4f}]")
    print(f"  win rate %:         {win_rate(runs):.2f}     CI [{wr[0]:.2f}, {wr[1]:.2f}]")


def relic_buyers_win_vs_non(
    runs: list[dict[str, Any]], relic: str
) -> tuple[int, int, int, int]:
    """Returns (wins_buyers, n_buyers, wins_non, n_non)."""
    wb = nb = wn = nn = 0
    for r in runs:
        picked = r.get("relics_picked") or {}
        bought = int(picked.get(relic, 0)) > 0
        v = bool(r.get("victory"))
        if bought:
            nb += 1
            if v:
                wb += 1
        else:
            nn += 1
            if v:
                wn += 1
    return wb, nb, wn, nn


def fdr_relic_table(runs: list[dict[str, Any]], min_buys: int) -> None:
    relics: set[str] = set()
    for r in runs:
        for name in (r.get("relics_picked") or {}).keys():
            relics.add(name)
    rows: list[tuple[str, float]] = []
    details: list[tuple[str, int, int, int, int, float, float]] = []
    for rel in sorted(relics):
        wb, nb, wn, nn = relic_buyers_win_vs_non(runs, rel)
        if nb < min_buys or nn < 1:
            continue
        p = two_proportion_z_p_value(wb, nb, wn, nn)
        rows.append((rel, p))
        p_buy = 100.0 * wb / nb
        p_non = 100.0 * wn / nn
        details.append((rel, nb, wb, nn, wn, p_buy - p_non, p))
    if not rows:
        print(f"\nFDR: no relics with ≥{min_buys} buyers and ≥1 non-buyer.")
        return
    sig = benjamini_hochberg_fdr(rows, alpha=0.05)
    print(
        f"\nRelic buyers vs non-buyers (two-proportion z-test, FDR 5% BH; min {min_buys} buyers):"
    )
    print(
        f"{'relic':<28} {'n_b':>5} {'win%':>7} {'n_nb':>5} {'win%':>7} {'Δ%':>7} {'p':>8} {'FDR':>5}"
    )
    for rel, nb, wb, nn, wn, _, p in sorted(details, key=lambda t: t[6]):
        p_b = 100.0 * wb / nb if nb else 0.0
        p_n = 100.0 * wn / nn if nn else 0.0
        delta = p_b - p_n
        print(
            f"{rel:<28} {nb:>5} {p_b:>6.1f}% {nn:>5} {p_n:>6.1f}% {delta:>+6.1f}% {p:>8.4f} {'yes' if sig.get(rel) else 'no':>5}"
        )


def stratify_ante_bin(antes: int, bins: list[int]) -> int:
    """Bins default [2,4]: 0 = antes<2, 1 = [2,4), 2 = >=4."""
    if not bins:
        return 0
    if antes < bins[0]:
        return 0
    for i in range(len(bins) - 1):
        if bins[i] <= antes < bins[i + 1]:
            return i + 1
    return len(bins)


def stratified_relic_summary(
    runs: list[dict[str, Any]], bins: list[int], min_in_bin: int
) -> None:
    """Win rate of buyers vs non-buyers within each antes_cleared bin."""
    if not bins:
        bins = [2, 4]
    print(
        f"\nStratified by antes_cleared: <{bins[0]} | "
        + " | ".join(f"[{bins[i]},{bins[i+1]})" for i in range(len(bins) - 1))
        + f" | >={bins[-1]} (min {min_in_bin} buyers per cell)"
    )

    relics: set[str] = set()
    for r in runs:
        for name in (r.get("relics_picked") or {}).keys():
            relics.add(name)

    def bin_label(b: int) -> str:
        if b == 0:
            return f"antes<{bins[0]}"
        if b >= len(bins):
            return f"antes>={bins[-1]}"
        return f"[{bins[b-1]},{bins[b]})"

    for rel in sorted(relics):
        printed_header = False
        for b in range(len(bins) + 1):
            sub = [
                r
                for r in runs
                if stratify_ante_bin(int(r.get("antes_cleared", 0)), bins) == b
            ]
            if len(sub) < min_in_bin:
                continue
            wb, nb, wn, nn = relic_buyers_win_vs_non(sub, rel)
            if nb < min_in_bin:
                continue
            if not printed_header:
                print(f"\n  {rel}:")
                printed_header = True
            p_b = 100.0 * wb / nb if nb else 0.0
            p_n = 100.0 * wn / nn if nn else 0.0
            print(
                f"    {bin_label(b):<14} n={len(sub):<5} buyers {nb:>4} win {p_b:>5.1f}% | non {nn:>4} win {p_n:>5.1f}%"
            )


def logistic_skeleton(runs: list[dict[str, Any]]) -> None:
    print("\n--- Logistic regression skeleton (optional sklearn) ---")
    try:
        import numpy as np
        from sklearn.linear_model import LogisticRegression
        from sklearn.preprocessing import StandardScaler
    except ImportError:
        print(
            "Install scikit-learn + numpy for `victory ~ features` demo, e.g.\n"
            "  pip install numpy scikit-learn"
        )
        return
    X_list = []
    y = []
    for r in runs:
        y.append(1 if r.get("victory") else 0)
        X_list.append(
            [
                float(r.get("antes_cleared", 0)),
                float(r.get("total_score", 0)) / 1e6,
                float(r.get("relics_bought", 0)),
                float(r.get("plays_used", 0)),
            ]
        )
    X = np.array(X_list, dtype=np.float64)
    y = np.array(y, dtype=np.int32)
    if len(np.unique(y)) < 2:
        print("Constant outcome — skip logistic fit.")
        return
    Xs = StandardScaler().fit_transform(X)
    m = LogisticRegression(max_iter=500)
    m.fit(Xs, y)
    names = ["antes_cleared", "total_score/1e6", "relics_bought", "plays_used"]
    print("Coefficients (standardized X), predict victory:")
    for name, coef in zip(names, m.coef_[0]):
        print(f"  {name:<22} {coef:+.4f}")
    print(f"  intercept              {m.intercept_[0]:+.4f}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("jsonl", help="Path to runs JSONL from mahjuro bot --output-runs")
    ap.add_argument("--bootstrap", type=int, default=0, help="Bootstrap iterations for mean antes / win rate")
    ap.add_argument("--seed", type=int, default=None, help="RNG seed for bootstrap")
    ap.add_argument("--fdr", action="store_true", help="Run FDR table for relic buyers vs non-buyers")
    ap.add_argument("--relic-min-buys", type=int, default=20, help="Min buyers to include relic in FDR table")
    ap.add_argument(
        "--stratify-relics",
        action="store_true",
        help="Stratify relic win rates by antes_cleared bins",
    )
    ap.add_argument(
        "--ante-bins",
        default="2,4",
        help="Comma-separated upper bounds for antes_cleared bins (default 2,4 → <2, [2,4), >=4)",
    )
    ap.add_argument("--logistic", action="store_true", help="Try sklearn logistic regression demo")
    args = ap.parse_args()

    runs = load_runs(args.jsonl)
    print(f"Loaded {len(runs)} runs from {args.jsonl!r}")

    wins = sum(1 for r in runs if r.get("victory"))
    n = len(runs)
    wr = 100.0 * wins / n if n else 0.0
    ci = wilson_95_pct(wins, n)
    ci_s = f" [{ci[0]:.1f}–{ci[1]:.1f}%]" if ci else ""
    print(f"Win rate: {wr:.2f}%{ci_s} (Wilson 95%)")

    mean_antes = sum(float(r.get("antes_cleared", 0)) for r in runs) / n if n else 0.0
    print(f"Mean antes_cleared: {mean_antes:.4f}")

    death_hazard_table(runs)

    if args.bootstrap > 0:
        bootstrap_means(runs, args.bootstrap, args.seed)

    if args.fdr:
        fdr_relic_table(runs, args.relic_min_buys)

    if args.stratify_relics:
        bins = [int(x.strip()) for x in args.ante_bins.split(",") if x.strip()]
        if len(bins) < 1:
            bins = [2, 4]
        stratified_relic_summary(runs, sorted(bins), min_in_bin=max(5, args.relic_min_buys // 4))

    if args.logistic:
        logistic_skeleton(runs)

    return 0


if __name__ == "__main__":
    sys.exit(main())
