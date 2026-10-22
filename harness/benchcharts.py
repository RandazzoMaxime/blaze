"""benchcharts.py: the README's benchmark charts, as SVG, from the results benchreport.py writes.

benchreport.py is the one caller: every time it writes a table into the README it writes the
chart of the same numbers next to it, so a chart is never older than its table. Each chart is
two files, `NAME-light.svg` and `NAME-dark.svg` under docs/benchmarks/charts, which the README
shows with `<picture>` by the reader's colour scheme. Hand-written SVG, standard library only:
nothing to pin, and a change of numbers is a readable diff.

  edits     how many times faster than incremental `cargo build` an edit builds, per project and
            platform: the median over the kinds of edit, the line from the slowest kind to the fastest
  memory    the most memory the work of an edit held at once, ra-jit (the daemon, its workers,
            the command), per project and platform, with cargo's (cargo, rustc, the linker) as
            the notch in each bar
  runtime   how long the program ra-jit's LLVM built takes against `cargo build --release`'s,
            per benchmark and platform, and the geometric mean

Each row has a bar per platform (benchplatforms.PLATFORMS, in that order, darkest first), and a
platform with no numbers for the row says so in place of its bar. Each platform is its own
machine, which the subtitle names: a bar is comparable with the bars of its own tone.

The look is the /bench page's: greys only.
"""
import math, os

from benchplatforms import NAMES

HERE = os.path.dirname(os.path.abspath(__file__))
CHARTS = os.path.join(os.path.dirname(HERE), "docs/benchmarks/charts")
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', Helvetica, Arial, sans-serif"
# Ink by scheme: a tone per platform, text, quieter text, grid lines, and the page behind the
# chart, GitHub's, for the notch in a bar (site/src/styles/tokens.css, and its grey ladder read
# the other way for a light page).
SCHEMES = {
    "light": dict(tones=("#1f1f20", "#73726f", "#b4b3af"), ink="#1f1f20", dim="#6b6a68", grid="#e4e3df", rule="#b3b2af", page="#ffffff"),
    "dark": dict(tones=("#ebeae6", "#a3a29f", "#5f5e5c"), ink="#ebeae6", dim="#929190", grid="#28282a", rule="#57575a", page="#0d1117"),
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
    (tone, value, low, high, text, notch): `tone` an index into a scheme's tones, a bar from the
    axis's base to `value`, a line from `low` to `high` when given, a notch at `notch` when given,
    and `text` after it; with `value` None, `text` alone, quieter, where the bar would start.
    `legend` is (name, tone), a tone of "notch" for the notch's key. The result is the SVG for
    each scheme."""
    label_w = max(text_width(label, 12.5) for label, _, _ in rows) + 16
    bars_per_row = max(len(bars) for _, _, bars in rows)
    row_h = ROW if bars_per_row == 1 else ROW + (bars_per_row - 1) * (BAR + 2) + 6
    lines = wrap(subtitle, 12, WIDTH - 24) if subtitle else []
    legend_y = 44 + 17 * len(lines)
    top = legend_y + (24 if legend else 4)
    x0, x1 = 12 + label_w, WIDTH - 110
    scale = Log(lo, hi, x0, x1)
    base = reference if reference else lo
    height = top + len(rows) * row_h + 44
    out = {}
    for scheme, c in SCHEMES.items():
        tone = c["tones"]
        svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" '
               f'font-family="{FONT}" role="img" aria-label="{esc(title)}">',
               f'<title>{esc(title)}</title>',
               f'<text x="12" y="22" font-size="15" font-weight="600" fill="{c["ink"]}">{esc(title)}</text>']
        svg += [f'<text x="12" y="{41 + 17 * k}" font-size="12" fill="{c["dim"]}">{esc(line)}</text>' for k, line in enumerate(lines)]
        at = 12
        for name, t in legend:
            if t == "notch":
                svg.append(f'<rect x="{at}" y="{legend_y - 9}" width="18" height="10" rx="2" fill="{c["dim"]}"/>')
                svg.append(f'<rect x="{at + 8}" y="{legend_y - 10}" width="2" height="12" fill="{c["page"]}"/>')
                at += 8
            else:
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
            if i and bars_per_row > 1:
                svg.append(f'<line x1="12" y1="{y:.1f}" x2="{WIDTH - 12}" y2="{y:.1f}" stroke="{c["grid"]}"/>')
            first = y + (row_h - len(bars) * BAR - (len(bars) - 1) * 2) / 2
            weight = ' font-weight="600"' if bold else ""
            svg.append(f'<text x="{label_w:.1f}" y="{y + row_h / 2 + 4.5:.1f}" font-size="12.5" text-anchor="end" fill="{c["ink"]}"{weight}>{esc(label)}</text>')
            for j, (t, value, low, high, said, *notch) in enumerate(bars):
                by = first + j * (BAR + 2)
                if value is None:
                    svg.append(f'<text x="{scale(base) + 4:.1f}" y="{by + BAR - 2:.1f}" font-size="11" fill="{c["dim"]}">{esc(said)}</text>')
                    continue
                a, b = sorted((scale(base), scale(min(max(value, lo), hi))))
                svg.append(f'<rect x="{a:.1f}" y="{by:.1f}" width="{max(b - a, 1.5):.1f}" height="{BAR}" rx="2" fill="{tone[t]}"/>')
                if notch and notch[0] is not None and lo < notch[0] < value:
                    svg.append(f'<rect x="{scale(notch[0]) - 1:.1f}" y="{by - 1:.1f}" width="2" height="{BAR + 2}" fill="{c["page"]}"/>')
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


def median(values):
    r = sorted(values)
    return r[len(r) // 2] if len(r) % 2 else math.sqrt(r[len(r) // 2 - 1] * r[len(r) // 2])


def platform_legend(measured):
    return [(name, NAMES.index(name)) for name in NAMES if name in measured]


def edits(rows, where):
    """`rows`: (project, {platform: [cargo seconds / ra-jit seconds of each kind of edit]}).
    `where` names each platform's machine. The chart's alt text is its own summary."""
    rows = [(p, {k: sorted(v) for k, v in by.items() if v}) for p, by in rows]
    rows = [(p, by) for p, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    lo, hi = bounds([x for _, by in rows for r in by.values() for x in r], floor=1)
    bars = [(p, False, [(NAMES.index(k), median(by[k]), by[k][0], by[k][-1], times(median(by[k]))) if k in by else
                        (NAMES.index(k), None, None, None, f"{k}: not measured")
                        for k in NAMES if k in measured]) for p, by in rows]
    write("edits", chart("Edit, then build: how many times faster than cargo",
                         f"Incremental cargo build against cargo ra-jit build, both warm, on each platform's own machine. The bar is "
                         f"the median over the kinds of edit, the line spans the slowest to the fastest kind. {where}",
                         bars, "times faster than incremental cargo build (log scale)", lo, hi, lambda v: f"{v:g}×",
                         legend=platform_legend(measured)))
    return "; ".join(f"{p}: " + ", ".join(f"{k} {times(median(r))}" for k, r in by.items()) for p, by in rows)


def memory(rows, where):
    """`rows`: (project, {platform: (cargo's peak MB, ra-jit's peak MB)}) during an edit."""
    rows = [(p, {k: v for k, v in by.items() if v[0] and v[1]}) for p, by in rows]
    rows = [(p, by) for p, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    lo, hi = bounds([v for _, by in rows for pair in by.values() for v in pair], pad=1.6)
    bars = [(p, False, [(NAMES.index(k), by[k][1], None, None, f"{gigabytes(by[k][1])}, cargo {gigabytes(by[k][0])}", by[k][0]) if k in by else
                        (NAMES.index(k), None, None, None, f"{k}: not measured")
                        for k in NAMES if k in measured]) for p, by in rows]
    write("memory", chart("Memory while an edit to one function builds",
                          f"The most every process doing the work held at once: the bar is ra-jit's, with the daemon, which keeps the "
                          f"workspace loaded; the notch is cargo's (cargo, rustc, the linker). {where}",
                          bars, "peak memory of every process at once (log scale)", lo, hi, lambda v: gigabytes(v).replace(".0 ", " "),
                          legend=platform_legend(measured) + [("cargo's peak", "notch")]))
    return "; ".join(f"{p}: " + ", ".join(f"{k} {gigabytes(b)} for ra-jit, {gigabytes(a)} for cargo" for k, (a, b) in by.items())
                     for p, by in rows)


def runtime(rows, geomeans, where):
    """`rows`: (benchmark, {platform: ra-jit's time / release's}); `geomeans`: {platform: geometric mean}."""
    rows = [(b, {k: v for k, v in by.items() if v}) for b, by in rows]
    rows = [(b, by) for b, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    values = [v for _, by in rows for v in by.values()] + [v for v in geomeans.values() if v]
    lo, hi = bounds(values + [1.0], pad=1.3)
    cells = lambda by: [(NAMES.index(k), by[k], None, None, f"{by[k]:.2f}×") if by.get(k) else
                        (NAMES.index(k), None, None, None, f"{k}: not measured") for k in NAMES if k in measured]
    bars = [(b, False, cells(by)) for b, by in rows]
    if any(geomeans.values()):
        bars.append(("Geometric mean", True, cells(geomeans)))
    write("runtime", chart("How fast the program ra-jit builds runs",
                           f"Time of each benchmark with ra-jit's LLVM build ÷ its time with cargo build --release on the same "
                           f"machine: 1× is as fast, to the right is slower. {where}",
                           bars, "time ÷ cargo build --release (log scale)", lo, hi, lambda v: f"{v:g}×",
                           reference=1.0, reference_label="as fast as release", legend=platform_legend(measured)))
    return "; ".join(f"{k} geometric mean {v:.2f}×" for k, v in geomeans.items() if v) or \
        "; ".join(f"{b}: " + ", ".join(f"{k} {v:.2f}×" for k, v in by.items()) for b, by in rows)
