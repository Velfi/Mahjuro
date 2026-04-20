from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

os.environ.setdefault("MPLCONFIGDIR", tempfile.mkdtemp(prefix="mahjuro-mpl-"))

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


ROOT = Path(__file__).resolve().parents[1]
DATA_PATH = ROOT / "docs" / "bot_balance_runs.json"


def avg(total: float, runs: int) -> float:
    if runs == 0:
        return 0.0
    return float(total) / float(runs)


def legacy_snapshot(payload: dict) -> dict:
    aggregate = payload.get("aggregate", {})
    mode = payload.get("mode", {})
    runs = int(aggregate.get("runs", payload.get("runs", 0)))

    return {
        "slug": "legacy_snapshot",
        "label": (
            f"Base {mode.get('base_target', '?')}\n"
            f"Scale {float(mode.get('target_scaling', 0.0)):.2f}\n"
            f"P{mode.get('starting_plays', '?')} D{mode.get('starting_discards', '?')} G{mode.get('starting_gold', '?')}\n"
            f"({runs} runs)"
        ),
        "runs": runs,
        "win_rate": avg(100.0 * aggregate.get("victories", 0), runs),
        "avg_blinds": avg(aggregate.get("blinds_cleared_total", 0), runs),
        "avg_antes": avg(aggregate.get("antes_cleared_total", 0), runs),
        "avg_total_score_m": avg(aggregate.get("total_score", 0), runs) / 1_000_000.0,
        "avg_plays": avg(aggregate.get("total_plays", 0), runs),
        "avg_discards": avg(aggregate.get("total_discards", 0), runs),
        "avg_skips": avg(aggregate.get("total_blinds_skipped", 0), runs),
        "avg_relics": avg(aggregate.get("total_relics_bought", 0), runs),
        "avg_gold_spent": avg(aggregate.get("total_gold_spent", 0), runs),
        "avg_gold_earned": avg(
            aggregate.get("total_gold_from_clears", 0) + aggregate.get("total_gold_from_skip_tags", 0),
            runs,
        ),
        "clear_base": avg(aggregate.get("total_gold_from_clear_base", 0), runs),
        "clear_plays": avg(aggregate.get("total_gold_from_unused_plays", 0), runs),
        "clear_interest": avg(aggregate.get("total_gold_from_interest", 0), runs),
        "clear_relics": avg(aggregate.get("total_gold_from_clear_relics", 0), runs),
        "avg_final_gold": avg(aggregate.get("total_final_gold", 0), runs),
        "deaths_by_ante": aggregate.get("deaths_by_ante", {}),
        "overscore_by_slot": aggregate.get("overscore_by_slot", {}),
        "cleared_by_slot": aggregate.get("cleared_by_slot", {}),
    }


def load_runs() -> list[dict]:
    with DATA_PATH.open() as f:
        payload = json.load(f)

    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict) and "aggregate" in payload:
        return [legacy_snapshot(payload)]
    raise TypeError(f"Unsupported bot balance payload shape: {type(payload).__name__}")


def add_bar_labels(ax) -> None:
    for container in ax.containers:
        ax.bar_label(container, fmt="%.1f", padding=2, fontsize=8)


def run_value(run: dict, key: str, default: float = 0.0) -> float:
    value = run.get(key)
    if value is None:
        return default
    return float(value)


def plot_summary(runs: list[dict], out_path: Path) -> None:
    labels = [r["label"] for r in runs]
    xs = list(range(len(runs)))

    fig, axes = plt.subplots(3, 2, figsize=(16, 14), constrained_layout=True)
    fig.suptitle("Mahjuro Bot Balance Snapshots", fontsize=18, fontweight="bold")

    ax = axes[0][0]
    width = 0.38
    ax.bar(
        [i - width / 2 for i in xs],
        [run_value(r, "win_rate") for r in runs],
        width=width,
        label="Win rate %",
        color="#c96f3b",
    )
    ax.bar(
        [i + width / 2 for i in xs],
        [run_value(r, "avg_antes") for r in runs],
        width=width,
        label="Avg antes",
        color="#4f6d4a",
    )
    ax.set_title("Difficulty")
    ax.set_xticks(xs, labels)
    ax.legend()
    add_bar_labels(ax)

    ax = axes[0][1]
    scores = [run_value(r, "avg_total_score_m") for r in runs]
    ax.plot(xs, scores, marker="o", linewidth=2.5, color="#355c7d")
    ax.set_title("Average Total Score (Millions)")
    ax.set_xticks(xs, labels)
    ax.set_ylabel("Millions")
    for i, y in enumerate(scores):
        ax.annotate(f"{y:.1f}", (i, y), textcoords="offset points", xytext=(0, 6), ha="center", fontsize=8)

    ax = axes[1][0]
    ax.plot(xs, [run_value(r, "avg_plays") for r in runs], marker="o", linewidth=2.0, label="Avg plays used", color="#6c5b7b")
    ax.plot(xs, [run_value(r, "avg_discards") for r in runs], marker="o", linewidth=2.0, label="Avg discards used", color="#f08a5d")
    ax.set_title("Action Economy")
    ax.set_xticks(xs, labels)
    ax.legend()

    ax = axes[1][1]
    ax.plot(xs, [run_value(r, "avg_blinds") for r in runs], marker="o", linewidth=2.0, label="Avg blinds cleared", color="#2a9d8f")
    ax.plot(xs, [run_value(r, "avg_skips") for r in runs], marker="o", linewidth=2.0, label="Avg blinds skipped", color="#e76f51")
    ax.set_title("Blind Pace")
    ax.set_xticks(xs, labels)
    ax.legend()

    ax = axes[2][0]
    ax.bar([i - 0.18 for i in xs], [run_value(r, "avg_relics") for r in runs], width=0.36, label="Avg relics", color="#8ab17d")
    ax.bar([i + 0.18 for i in xs], [run_value(r, "avg_gold_spent") for r in runs], width=0.36, label="Avg gold spent", color="#bc6c25")
    ax.set_title("Shop Pressure")
    ax.set_xticks(xs, labels)
    ax.legend()
    add_bar_labels(ax)

    ax = axes[2][1]
    ax.bar([i - 0.18 for i in xs], [run_value(r, "avg_gold_earned") for r in runs], width=0.36, label="Avg gold earned", color="#457b9d")
    ax.bar([i + 0.18 for i in xs], [run_value(r, "avg_final_gold") for r in runs], width=0.36, label="Avg final gold", color="#a8dadc")
    ax.set_title("Economy Outcomes")
    ax.set_xticks(xs, labels)
    ax.legend()
    add_bar_labels(ax)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out_path, dpi=180)
    plt.close(fig)


def plot_survival_heatmap(runs: list[dict], out_path: Path) -> None:
    ante_keys = sorted({int(ante) for run in runs for ante in run.get("deaths_by_ante", {}).keys()})
    if not ante_keys:
        return

    labels = [r["label"] for r in runs]
    matrix = []
    for run in runs:
        deaths = run.get("deaths_by_ante", {})
        run_count = max(int(run.get("runs", 0)), 1)
        matrix.append([(100.0 * deaths.get(str(ante), 0)) / run_count for ante in ante_keys])

    data = np.array(matrix)
    fig_h = max(5.5, len(runs) * 0.65)
    fig, ax = plt.subplots(figsize=(16, fig_h), constrained_layout=True)
    image = ax.imshow(data, aspect="auto", cmap="YlOrRd")

    ax.set_title("Death Rate By Ante (% of Runs)")
    ax.set_xticks(range(len(ante_keys)), [f"Ante {ante}" for ante in ante_keys])
    ax.set_yticks(range(len(labels)), labels)

    for row_idx in range(data.shape[0]):
        for col_idx in range(data.shape[1]):
            value = data[row_idx, col_idx]
            if value > 0:
                text_color = "black" if value < 18 else "white"
                ax.text(col_idx, row_idx, f"{value:.1f}", ha="center", va="center", fontsize=8, color=text_color)

    cbar = fig.colorbar(image, ax=ax, shrink=0.9)
    cbar.set_label("% of runs ending on ante")

    fig.savefig(out_path, dpi=180)
    plt.close(fig)


def plot_snapshot_tradeoffs(runs: list[dict], out_path: Path) -> None:
    fig, ax = plt.subplots(figsize=(14, 8), constrained_layout=True)

    scores = [run_value(r, "avg_total_score_m") for r in runs]
    win_rates = [run_value(r, "win_rate") for r in runs]
    point_sizes = [max(60.0, run_value(r, "avg_final_gold") * 18.0 + 60.0) for r in runs]
    colors = [run_value(r, "avg_antes") for r in runs]

    scatter = ax.scatter(scores, win_rates, s=point_sizes, c=colors, cmap="viridis", alpha=0.85, edgecolors="#1f1f1f", linewidths=0.8)
    ax.set_title("Snapshot Tradeoffs")
    ax.set_xlabel("Average total score (millions)")
    ax.set_ylabel("Win rate %")

    for run, x, y in zip(runs, scores, win_rates):
        ax.annotate(run["label"], (x, y), textcoords="offset points", xytext=(6, 6), fontsize=8)

    cbar = fig.colorbar(scatter, ax=ax)
    cbar.set_label("Average antes cleared")
    ax.grid(alpha=0.25)

    fig.savefig(out_path, dpi=180)
    plt.close(fig)
def plot_deaths_by_ante(runs: list[dict], out_path: Path) -> None:
    ante_keys = sorted({int(ante) for run in runs for ante in run.get("deaths_by_ante", {}).keys()})
    labels = [r["label"] for r in runs]
    xs = list(range(len(ante_keys)))

    fig, ax = plt.subplots(figsize=(14, 8), constrained_layout=True)
    for run in runs:
        data = run.get("deaths_by_ante", {})
        ys = [data.get(str(ante), 0) for ante in ante_keys]
        ax.plot(xs, ys, marker="o", linewidth=2.2, label=run["label"])

    ax.set_title("Deaths By Ante Across Balance Snapshots")
    ax.set_xticks(xs, [f"Ante {ante}" for ante in ante_keys])
    ax.set_ylabel("Deaths")
    ax.legend(fontsize=9)

    fig.savefig(out_path, dpi=180)
    plt.close(fig)


def plot_economy(runs: list[dict], out_path: Path) -> None:
    econ_runs = [r for r in runs if "avg_gold_earned" in r]
    labels = [r["label"] for r in econ_runs]
    xs = list(range(len(econ_runs)))

    fig, axes = plt.subplots(1, 2, figsize=(14, 6), constrained_layout=True)

    ax = axes[0]
    base = [r["clear_base"] for r in econ_runs]
    plays = [r["clear_plays"] for r in econ_runs]
    interest = [r["clear_interest"] for r in econ_runs]
    relics = [r["clear_relics"] for r in econ_runs]
    ax.bar(xs, base, label="Base", color="#7aa874")
    ax.bar(xs, plays, bottom=base, label="Unused plays", color="#f2c14e")
    bottom2 = [a + b for a, b in zip(base, plays)]
    ax.bar(xs, interest, bottom=bottom2, label="Interest", color="#e27d60")
    bottom3 = [a + b + c for a, b, c in zip(base, plays, interest)]
    ax.bar(xs, relics, bottom=bottom3, label="Relics", color="#85dcb8")
    ax.set_title("Average Clear-Payout Breakdown")
    ax.set_xticks(xs, labels)
    ax.legend()

    ax = axes[1]
    ax.bar([i - 0.18 for i in xs], [r["avg_gold_earned"] for r in econ_runs], width=0.36, label="Avg gold earned", color="#457b9d")
    ax.bar([i + 0.18 for i in xs], [r["avg_final_gold"] for r in econ_runs], width=0.36, label="Avg final gold", color="#a8dadc")
    ax.set_title("Economy Outcomes")
    ax.set_xticks(xs, labels)
    ax.legend()
    add_bar_labels(ax)

    fig.savefig(out_path, dpi=180)
    plt.close(fig)


BLIND_ORDER = {"Small Blind": 0, "Big Blind": 1, "Boss Blind": 2}
BLIND_SHORT = {"Small Blind": "S", "Big Blind": "B", "Boss Blind": "X"}


def slot_sort_key(slot: str) -> tuple[int, int]:
    ante_str, _, blind = slot.partition("-")
    try:
        ante = int(ante_str)
    except ValueError:
        ante = 99
    return (ante, BLIND_ORDER.get(blind, 99))


def plot_surplus_per_blind(runs: list[dict], out_path: Path) -> None:
    surplus_runs = [r for r in runs if r.get("cleared_by_slot")]
    if not surplus_runs:
        return

    all_slots = sorted(
        {slot for r in surplus_runs for slot in r.get("cleared_by_slot", {})},
        key=slot_sort_key,
    )
    xs = list(range(len(all_slots)))
    tick_labels = [
        f"A{int(s.split('-', 1)[0])}{BLIND_SHORT.get(s.split('-', 1)[1], '?')}"
        for s in all_slots
    ]

    fig, ax = plt.subplots(figsize=(max(14, len(all_slots) * 0.35), 8), constrained_layout=True)
    fig.suptitle("Average Score Surplus Per Blind", fontsize=16, fontweight="bold")

    for run in surplus_runs:
        overscore = run.get("overscore_by_slot", {})
        cleared = run.get("cleared_by_slot", {})
        ys = []
        for slot in all_slots:
            count = cleared.get(slot, 0)
            ys.append(overscore.get(slot, 0) / count if count else np.nan)
        ax.plot(xs, ys, marker="o", linewidth=2.0, label=run["label"])

    ax.set_yscale("symlog", linthresh=1000)
    ax.set_xticks(xs, tick_labels, rotation=45, ha="right", fontsize=8)
    ax.set_ylabel("Avg surplus (score over target, log scale)")
    ax.set_xlabel("Ante / Blind (S=Small, B=Big, X=Boss)")
    ax.grid(alpha=0.25, which="both")
    ax.legend(fontsize=9)

    fig.savefig(out_path, dpi=180)
    plt.close(fig)


def main() -> None:
    runs = load_runs()
    docs = ROOT / "docs"
    plot_summary(runs, docs / "bot_balance_summary.png")
    plot_deaths_by_ante(runs, docs / "bot_balance_deaths_by_ante.png")
    plot_economy(runs, docs / "bot_balance_economy.png")
    plot_survival_heatmap(runs, docs / "bot_balance_survival_heatmap.png")
    plot_snapshot_tradeoffs(runs, docs / "bot_balance_snapshot_tradeoffs.png")
    plot_surplus_per_blind(runs, docs / "bot_balance_surplus_per_blind.png")


if __name__ == "__main__":
    main()
