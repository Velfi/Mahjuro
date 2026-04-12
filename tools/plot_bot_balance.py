from __future__ import annotations

import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


ROOT = Path(__file__).resolve().parents[1]
DATA_PATH = ROOT / "docs" / "bot_balance_runs.json"


def load_runs() -> list[dict]:
    with DATA_PATH.open() as f:
        return json.load(f)


def add_bar_labels(ax) -> None:
    for container in ax.containers:
        ax.bar_label(container, fmt="%.1f", padding=2, fontsize=8)


def plot_summary(runs: list[dict], out_path: Path) -> None:
    labels = [r["label"] for r in runs]
    xs = list(range(len(runs)))

    fig, axes = plt.subplots(2, 2, figsize=(14, 10), constrained_layout=True)
    fig.suptitle("Mahjuro Bot Balance Snapshots", fontsize=18, fontweight="bold")

    ax = axes[0][0]
    width = 0.38
    ax.bar([i - width / 2 for i in xs], [r["win_rate"] for r in runs], width=width, label="Win rate %", color="#c96f3b")
    ax.bar([i + width / 2 for i in xs], [r["avg_antes"] for r in runs], width=width, label="Avg antes", color="#4f6d4a")
    ax.set_title("Difficulty")
    ax.set_xticks(xs, labels)
    ax.legend()
    add_bar_labels(ax)

    ax = axes[0][1]
    scores = [r["avg_total_score_m"] for r in runs]
    ax.plot(xs, scores, marker="o", linewidth=2.5, color="#355c7d")
    ax.set_title("Average Total Score (Millions)")
    ax.set_xticks(xs, labels)
    ax.set_ylabel("Millions")
    for i, y in enumerate(scores):
        ax.annotate(f"{y:.1f}", (i, y), textcoords="offset points", xytext=(0, 6), ha="center", fontsize=8)

    ax = axes[1][0]
    ax.plot(xs, [r["avg_plays"] for r in runs], marker="o", linewidth=2.0, label="Avg plays used", color="#6c5b7b")
    ax.plot(xs, [r["avg_discards"] for r in runs], marker="o", linewidth=2.0, label="Avg discards used", color="#f08a5d")
    ax.set_title("Action Economy")
    ax.set_xticks(xs, labels)
    ax.legend()

    ax = axes[1][1]
    ax.plot(xs, [r["avg_blinds"] for r in runs], marker="o", linewidth=2.0, label="Avg blinds cleared", color="#2a9d8f")
    ax.plot(xs, [r["avg_skips"] for r in runs], marker="o", linewidth=2.0, label="Avg blinds skipped", color="#e76f51")
    ax.set_title("Blind Pace")
    ax.set_xticks(xs, labels)
    ax.legend()

    out_path.parent.mkdir(parents=True, exist_ok=True)
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


def main() -> None:
    runs = load_runs()
    docs = ROOT / "docs"
    plot_summary(runs, docs / "bot_balance_summary.png")
    plot_deaths_by_ante(runs, docs / "bot_balance_deaths_by_ante.png")
    plot_economy(runs, docs / "bot_balance_economy.png")


if __name__ == "__main__":
    main()
