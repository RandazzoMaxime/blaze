"""benchcharts.py: the README's benchmark charts, as SVG, from the results benchreport.py writes.

benchreport.py is the one caller: every time it writes a table into the README it writes the
chart of the same numbers next to it, so a chart is never older than its table. Each chart is
two files, `NAME-light.svg` and `NAME-dark.svg` under docs/benchmarks/charts, which the README
shows with `<picture>` by the reader's colour scheme. Hand-written SVG, standard library only:
nothing to pin, and a change of numbers is a readable diff.

  edits     how many times faster than incremental `cargo build` an edit builds, per project:
            the median over the kinds of edit, the line from the slowest kind to the fastest
  memory    the most memory the work of an edit held at once, ra-jit (the daemon, its workers,
            the command) against cargo (cargo, rustc, the linker), per project
  runtime   how long the program ra-jit's LLVM built takes against `cargo build --release`'s,
            per benchmark, and the geometric mean

The look is the /bench page's: greys only, ra-jit the strongest ink, cargo the lighter one.
"""
import math, os

HERE = os.path.dirname(os.path.abspath(__file__))
CHARTS = os.path.join(os.path.dirname(HERE), "docs/benchmarks/charts")
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', Helvetica, Arial, sans-serif"
# Ink by scheme: ra-jit, cargo, text, quieter text, grid lines (site/src/styles/tokens.css, and
# its grey ladder read the other way for a light page).
SCHEMES = {
    "light": dict(ours="#1f1f20", other="#a3a29f", ink="#1f1f20", dim="#6b6a68", grid="#e4e3df", rule="#b3b2af"),
    "dark": dict(ours="#ebeae6", other="#6f6e6c", ink="#ebeae6", dim="#929190", grid="#28282a", rule="#57575a"),
}
WIDTH, ROW, BAR = 760, 24, 12


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def text_width(text, size):
    """About how wide `text` is in the sans-serif fonts above: enough to lay out a column."""
    narrow = sum(c in "ijlt.,:;|!' ()1" for c in text)
    return size * (0.58 * (len(text) - narrow) + 0.3 * narrow)


def wrap(text, size, width):
    lines, line = [], ""
    for word in text.split():
        if line and text_width(line + " " + word, size) > width:
            lines.append(line)
            line = word
        else:
            line = f"{line} {word}".strip()
    return lines + [line] if line else lines


class Log:
    """A logarithmic axis from `lo` to `hi` onto pixels `x0` to `x1`."""

    def __init__(self, lo, hi, x0, x1):
        self.lo, self.hi, self.x0, self.x1 = math.log10(lo), math.log10(hi), x0, x1

    def __call__(self, v):
        return self.x0 + (math.log10(v) - self.lo) / (self.hi - self.lo) * (self.x1 - self.x0)


def ticks(lo, hi, steps=None):
    """Round values between `lo` and `hi`, as many as a log axis has room for."""
    steps = steps or ((1, 2, 5) if hi / lo >= 6 else (1, 1.25, 1.5, 2, 3, 4, 5, 7))
    found, decade = [], 10 ** math.floor(math.log10(lo))
    while decade <= hi:
        found += [decade * s for s in steps if lo <= decade * s <= hi]
        decade *= 10
    return found


def bounds(values, pad=1.25, floor=None):
    """An axis a little wider than `values`: at the next tick out when they span a decade or
    more, else just padded."""
    lo, hi = min(values) / pad, max(values) * pad
    lo = min(lo, floor) if floor else lo
    if hi / lo < 10:
        return lo, hi
    out = ticks(lo / 10, hi * 10)
    return max([t for t in out if t <= lo] or [lo]), min([t for t in out if t >= hi] or [hi])


def chart(title, subtitle, rows, axis_label, lo, hi, tick_label, legend=(), reference=None, reference_label=None):
    """A horizontal bar chart on a log axis. `rows` is a list of (label, bold, bars), each bar
    (tone, value, low, high, text): a bar from the axis's base to `value`, a line from `low` to
    `high` when given, and `text` after it. The result is the SVG for each scheme."""
    label_w = max(text_width(label, 12.5) for label, _, _ in rows) + 16
    bars_per_row = max(len(bars) for _, _, bars in rows)
    row_h = ROW if bars_per_row == 1 else ROW + (bars_per_row - 1) * (BAR + 2)
    lines = wrap(subtitle, 12, WIDTH - 24) if subtitle else []
    legend_y = 44 + 17 * len(lines)
    top = legend_y + (24 if legend else 4)
    x0, x1 = 12 + label_w, WIDTH - 70
    scale = Log(lo, hi, x0, x1)
    base = reference if reference else lo
    height = top + len(rows) * row_h + 44
    out = {}
    for scheme, c in SCHEMES.items():
        tone = {"ours": c["ours"], "other": c["other"]}
        svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" '
               f'font-family="{FONT}" role="img" aria-label="{esc(title)}">',
               f'<title>{esc(title)}</title>',
               f'<text x="12" y="22" font-size="15" font-weight="600" fill="{c["ink"]}">{esc(title)}</text>']
        svg += [f'<text x="12" y="{41 + 17 * k}" font-size="12" fill="{c["dim"]}">{esc(line)}</text>' for k, line in enumerate(lines)]
        at = 12
        for name, t in legend:
            svg.append(f'<rect x="{at}" y="{legend_y - 9}" width="10" height="10" rx="2" fill="{tone[t]}"/>')
            svg.append(f'<text x="{at + 16}" y="{legend_y}" font-size="12" fill="{c["ink"]}">{esc(name)}</text>')
            at += 16 + text_width(name, 12) + 22
        plot_top, plot_bottom = top - 6, top + len(rows) * row_h
        for v in ticks(lo, hi):
            x = scale(v)
            svg.append(f'<line x1="{x:.1f}" y1="{plot_top}" x2="{x:.1f}" y2="{plot_bottom}" stroke="{c["grid"]}"/>')
            svg.append(f'<text x="{x:.1f}" y="{plot_bottom + 16}" font-size="11" text-anchor="middle" fill="{c["dim"]}">{esc(tick_label(v))}</text>')
        if reference:
            x = scale(reference)
            svg.append(f'<line x1="{x:.1f}" y1="{plot_top}" x2="{x:.1f}" y2="{plot_bottom}" stroke="{c["rule"]}" stroke-width="1.25"/>')
            if reference_label:
                svg.append(f'<text x="{x + 4:.1f}" y="{plot_top + 8}" font-size="11" fill="{c["dim"]}">{esc(reference_label)}</text>')
        svg.append(f'<text x="{(x0 + x1) / 2:.1f}" y="{plot_bottom + 34}" font-size="12" text-anchor="middle" fill="{c["dim"]}">{esc(axis_label)}</text>')
        for i, (label, bold, bars) in enumerate(rows):
            y = top + i * row_h
            first = y + (row_h - len(bars) * BAR - (len(bars) - 1) * 2) / 2
            weight = ' font-weight="600"' if bold else ""
            svg.append(f'<text x="{label_w:.1f}" y="{y + row_h / 2 + 4.5:.1f}" font-size="12.5" text-anchor="end" fill="{c["ink"]}"{weight}>{esc(label)}</text>')
            for j, (t, value, low, high, said) in enumerate(bars):
                by = first + j * (BAR + 2)
                a, b = sorted((scale(base), scale(min(max(value, lo), hi))))
                svg.append(f'<rect x="{a:.1f}" y="{by:.1f}" width="{max(b - a, 1.5):.1f}" height="{BAR}" rx="2" fill="{tone[t]}"/>')
                end = b if value >= base else a
                if low is not None and high is not None:
                    l, h = scale(max(low, lo)), scale(min(high, hi))
                    mid = by + BAR / 2
                    svg.append(f'<line x1="{l:.1f}" y1="{mid:.1f}" x2="{h:.1f}" y2="{mid:.1f}" stroke="{tone[t]}" stroke-width="1.25"/>')
                    svg += [f'<line x1="{e:.1f}" y1="{mid - 4:.1f}" x2="{e:.1f}" y2="{mid + 4:.1f}" stroke="{tone[t]}" stroke-width="1.25"/>' for e in (l, h)]
                    end = max(end, h) if value >= base else min(end, l)
                anchor, tx = ("start", end + 6) if value >= base else ("end", end - 6)
                svg.append(f'<text x="{tx:.1f}" y="{by + BAR - 2:.1f}" font-size="11.5" text-anchor="{anchor}" fill="{c["ink"]}"'
                           f'{weight} font-variant-numeric="tabular-nums">{esc(said)}</text>')
        svg.append("</svg>")
        out[scheme] = "\n".join(svg) + "\n"
    return out


def write(name, svgs):
    os.makedirs(CHARTS, exist_ok=True)
    for scheme, svg in svgs.items():
        with open(os.path.join(CHARTS, f"{name}-{scheme}.svg"), "w") as f:
            f.write(svg)


def picture(name, alt, prefix="docs/benchmarks/charts"):
    """The README's markup for a chart: the dark file for a dark scheme, the light one otherwise."""
    return (f'<picture><source media="(prefers-color-scheme: dark)" srcset="{prefix}/{name}-dark.svg">'
            f'<img alt="{esc(alt)}" src="{prefix}/{name}-light.svg" width="{WIDTH}"></picture>')


def times(v):
    return f"{v:.0f}×" if v >= 10 else f"{v:.1f}×"


def gigabytes(mb):
    return f"{mb / 1000:.1f} GB" if mb >= 1000 else f"{mb:.0f} MB"


def edits(rows, where):
    """`rows`: (project, [cargo seconds / ra-jit seconds of each kind of edit]). The chart's
    alt text is its own summary."""
    rows = [(p, sorted(r)) for p, r in rows if r]
    if not rows:
        return None
    med = lambda r: r[len(r) // 2] if len(r) % 2 else math.sqrt(r[len(r) // 2 - 1] * r[len(r) // 2])
    lo, hi = bounds([x for _, r in rows for x in r], floor=1)
    bars = [(p, False, [("ours", med(r), r[0], r[-1], times(med(r)))]) for p, r in rows]
    write("edits", chart("Edit, then build: how many times faster than cargo",
                         f"Incremental cargo build against cargo ra-jit build, both warm. The bar is the median over the kinds of edit, "
                         f"the line spans the slowest to the fastest kind. {where}",
                         bars, "times faster than incremental cargo build (log scale)", lo, hi, lambda v: f"{v:g}×"))
    return ", ".join(f"{p} {times(med(r))}" for p, r in rows)


def memory(rows, where):
    """`rows`: (project, cargo's peak MB, ra-jit's peak MB) during an edit."""
    rows = [r for r in rows if r[1] and r[2]]
    if not rows:
        return None
    lo, hi = bounds([v for _, a, b in rows for v in (a, b)], pad=1.6)
    bars = [(p, False, [("ours", b, None, None, gigabytes(b)), ("other", a, None, None, gigabytes(a))]) for p, a, b in rows]
    write("memory", chart("Memory while an edit to one function builds",
                          f"The most every process doing the work held at once. ra-jit's includes the daemon, which keeps the workspace loaded. {where}",
                          bars, "peak memory of every process at once (log scale)", lo, hi, lambda v: gigabytes(v).replace(".0 ", " "),
                          legend=[("ra-jit: daemon, workers, command", "ours"), ("cargo: cargo, rustc, linker", "other")]))
    return ", ".join(f"{p} {gigabytes(a)} for cargo, {gigabytes(b)} for ra-jit" for p, a, b in rows)


def runtime(rows, geomean, build, where):
    """`rows`: (benchmark, ra-jit's time / release's)."""
    rows = [r for r in rows if r[1]]
    if not rows:
        return None
    values = [v for _, v in rows] + ([geomean] if geomean else [])
    lo, hi = bounds(values + [1.0], pad=1.3)
    bars = [(b, False, [("ours", v, None, None, f"{v:.2f}×")]) for b, v in rows]
    if geomean:
        bars.append(("Geometric mean", True, [("ours", geomean, None, None, f"{geomean:.2f}×")]))
    write("runtime", chart("How fast the program ra-jit builds runs",
                           f"Time of each benchmark with ra-jit's LLVM build ({build}) ÷ its time with cargo build --release: "
                           f"1× is as fast, to the right is slower. {where}",
                           bars, "time ÷ cargo build --release (log scale)", lo, hi, lambda v: f"{v:g}×",
                           reference=1.0, reference_label="as fast as release"))
    return ", ".join(f"{b} {v:.2f}×" for b, v in rows)
