# Chart guidelines (Chronicle + UI)

Mahjuro charts follow [Duke Library — Top Ten Dos and Don'ts](https://guides.library.duke.edu/datavis/topten). Implementation lives in [`src/ui/chart_primitives.rs`](../../src/ui/chart_primitives.rs) and [`src/ui/chronicle_charts.rs`](../../src/ui/chronicle_charts.rs); palette tokens in [`color::chart`](../../src/render/theme.rs).

## Do

| Principle | How we apply it |
|-----------|-----------------|
| **Full axis for bars** | Vertical bars use a **zero baseline** and **linear** height (`score / max`). No squashing of tall bars. |
| **Line/sparklines may truncate Y** | KPI and blind-score sparklines autoscale (`autoscale_sparkline`) — acceptable for trends, not magnitudes. |
| **Simplify chrome** | Few or no gridlines on the career score history; KPI sparklines use a single baseline. |
| **Direct labels** | AVG / PEAK reference lines, axis ticks (`format_chart_axis_tick`), count + `%` on distribution rows, outcome legend swatches. |
| **Squint test** | Victory = `chart::POSITIVE`, defeat = `chart::NEGATIVE`, neutral magnitude = `chart::FILL`, highlight = `chart::HIGHLIGHT`. |
| **Consistent series** | Same outcome colors and `then` / `now` time axis on all time-series bar charts. |

## Don't

| Principle | How we avoid it |
|-----------|-----------------|
| **3D / blow-apart** | Flat quads only. |
| **>6 categorical colors** | ≤5 `color::chart` tokens; yaku **names** on pills, not rainbow bars. |
| **Rainbow / continuous hue scales** | No score→hue gradients. |
| **Visual math** | Precompute average, W·L, percentages; don't stack redundant encodings (e.g. bar + segmented tiles). |
| **Overload** | Split career pane into sections; sparklines in table cells stay minimal. |

## Adding a new chart

1. **Bar chart** → zero baseline, linear scale, optional `push_chart_y_axis`.
2. **Trend-only** → sparkline/line with autoscale OK.
3. **Categories** → reuse `POSITIVE` / `NEGATIVE` / `FILL`; add a legend if color appears without a nearby label.
4. **Time series** → one sample per run/event (even spacing); `push_chart_time_axis_labels` for chronology.
