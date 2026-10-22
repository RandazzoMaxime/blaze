"""benchcharts.py: the README's benchmark charts, as SVG, from the results benchreport.py writes.

benchreport.py is the one caller: every time it writes a table into the README it writes the
chart of the same numbers next to it, so a chart is never older than its table. Each chart is
two files, `NAME-light.svg` and `NAME-dark.svg` under docs/benchmarks/charts, which the README
shows with `<picture>` by the reader's colour scheme. Hand-written SVG, standard library only:
nothing to pin, and a change of numbers is a readable diff. The two files of a chart differ only
in their `<style>`, and the same numbers always give the same bytes.

  edits     how many times faster than incremental `cargo build` an edit builds, per project and
            platform: the median over the kinds of edit, the line from the slowest kind to the fastest
  memory    the most memory the work of an edit held at once, ra-jit (the daemon, its workers,
            the command), per project and platform, with cargo's (cargo, rustc, the linker) as
            a tick across each bar
  runtime   how long the program ra-jit's LLVM built takes against `cargo build --release`'s,
            per benchmark and platform, and the geometric mean

Each row has a bar per platform (benchplatforms.PLATFORMS, in that order), and a platform with
no numbers for the row says so in place of its bar. Each platform is its own machine, which the
footer names: a bar is comparable with the bars of its own colour.

The colours: a hue per platform, the same in every chart (blue, orange, teal: Okabe and Ito's
palette, apart under the common colour blindnesses), each tuned for the page it is drawn on, and
cargo always the neutral reference, the 1× line or the tick. The rest is GitHub's own greys, so
a chart sits in the README like its tables do.
"""
import math, os

from benchplatforms import NAMES

HERE = os.path.dirname(os.path.abspath(__file__))
CHARTS = os.path.join(os.path.dirname(HERE), "docs/benchmarks/charts")
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', Helvetica, Arial, sans-serif"
# By scheme: a colour per platform (in NAMES' order), text, quieter text, grid lines, the 1× line,
# the band behind every other row, and cargo's tick. The greys are GitHub's (Primer's fg.default,
# fg.muted, border.muted, canvas.subtle) for its light and dark pages.
SCHEMES = {
    "light": dict(platforms=("#1f6fd0", "#d8700a", "#12a07a"), ink="#1f2328", dim="#59636e", grid="#e3e7eb",
                  rule="#59636e", band="#f6f8fa", cargo="#1f2328"),
    "dark": dict(platforms=("#5aa9f5", "#f0913a", "#34c69a"), ink="#f0f6fc", dim="#9198a1", grid="#262c36",
                 rule="#9198a1", band="#151b23", cargo="#f0f6fc"),
}
WIDTH, PAD, BAR, GAP = 760, 16, 12, 3


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


def n(v):
    """A coordinate, to a tenth of a pixel."""
    return f"{v:.1f}".removesuffix(".0")


class Log:
    """A logarithmic axis from `lo` to `hi` onto pixels `x0` to `x1`."""

    def __init__(self, lo, hi, x0, x1):
        self.lo, self.hi, self.x0, self.x1 = math.log10(lo), math.log10(hi), x0, x1

    def __call__(self, v):
        return self.x0 + (math.log10(v) - self.lo) / (self.hi - self.lo) * (self.x1 - self.x0)


def steps(lo, hi):
    """The round values of each decade a log axis from `lo` to `hi` has room to mark."""
    return (1, 2, 5) if hi / lo >= 100 else (1, 1.5, 2, 3, 5, 7) if hi / lo >= 6 else (1, 1.25, 1.5, 2, 3, 4, 5, 7)


def ticks(lo, hi, marks=None):
    """Round values between `lo` and `hi`, as many as a log axis has room for."""
    marks = marks or steps(lo, hi)
    found, decade = [], 10 ** math.floor(math.log10(lo))
    while decade <= hi:
        found += [round(decade * s, 12) for s in marks if lo <= decade * s <= hi]
        decade *= 10
    return found


def bounds(values, pad=1.25, floor=None):
    """An axis a little wider than `values`: at the next tick out when they span a decade or
    more, else just padded."""
    lo, hi = min(values) / pad, max(values) * pad
    lo = min(lo, floor) if floor else lo
    if hi / lo < 10:
        return lo, hi
    out = ticks(lo / 10, hi * 10, steps(lo, hi))
    return max([t for t in out if t <= lo] or [lo]), min([t for t in out if t >= hi] or [hi])


def style(c):
    platforms = "".join(f".p{i}{{fill:{v}}}.w{i}{{stroke:{v}}}" for i, v in enumerate(c["platforms"]))
    return (f"<style>text{{font-family:{FONT};fill:{c['ink']}}}.d{{fill:{c['dim']}}}.b{{font-weight:600}}"
            f".v{{font-variant-numeric:tabular-nums}}.g{{stroke:{c['grid']}}}.r{{stroke:{c['rule']};stroke-width:1.25}}"
            f".z{{fill:{c['band']}}}.c{{fill:{c['cargo']}}}.o{{fill:none;stroke-width:1.25}}{platforms}</style>")


def chart(title, subtitle, rows, lo, hi, tick_label, notes="", legend=(), cargo_key=None, reference=None,
          reference_label=None, hints=None, axis_label=None):
    """A horizontal bar chart on a log axis. `rows` is a list of (label, bold, bars), each bar
    (platform, value, low, high, text, cargo): `platform` an index into NAMES, a bar from the axis's
    base (`reference`, else `lo`) to `value`, a line from `low` to `high` when given, cargo's tick
    at `cargo` when given, and `text` after it; with `value` None, an empty key and `text` where the
    bar would start. `legend` is the platforms to name, `cargo_key` what to call the tick, `hints`
    (left, right) what each side of `reference` means, `notes` the footer. The result is the SVG
    for each scheme."""
    label_w = max(text_width(label, 13) for label, _, _ in rows)
    value_w = max(text_width(said, 12) for _, _, bars in rows for _, value, *_, said, _ in bars if value is not None)
    x0, x1 = PAD + label_w + 16, WIDTH - PAD - value_w - 10
    scale = Log(lo, hi, x0, x1)
    base = reference or lo

    body, y = [f'<text x="{PAD}" y="26" font-size="17" class="b">{esc(title)}</text>'], 26
    for line in wrap(subtitle, 13, WIDTH - 2 * PAD):
        y += 19
        body.append(f'<text x="{PAD}" y="{y}" font-size="13" class="d">{esc(line)}</text>')
    keys = [(name, f'<rect x="{{x}}" y="{{y}}" width="12" height="12" rx="3" class="p{NAMES.index(name)}"/>', 18) for name in legend]
    if cargo_key:
        keys.append((cargo_key, '<rect x="{x}" y="{tick_y}" width="2.5" height="16" class="c"/>', 10))
    at, y = PAD, y + 22
    for name, swatch, w in keys:
        width = w + text_width(name, 12.5)
        if at > PAD and at + width > WIDTH - PAD:
            at, y = PAD, y + 22
        body.append(swatch.format(x=at, y=y - 10, tick_y=y - 12))
        body.append(f'<text x="{n(at + w)}" y="{y}" font-size="12.5">{esc(name)}</text>')
        at += width + 22

    top = y + (30 if reference_label else 16)
    heights = [len(bars) * BAR + (len(bars) - 1) * GAP + 16 for _, _, bars in rows]
    bottom = top + sum(heights)
    grid, marks, labels = [], [], []
    for v in ticks(lo, hi):
        x = n(scale(v))
        grid.append(f'<line x1="{x}" y1="{top}" x2="{x}" y2="{bottom}" class="g"/>')
        labels.append(f'<text x="{x}" y="{bottom + 17}" font-size="11.5" text-anchor="middle" class="d v">{esc(tick_label(v))}</text>')
    row_y = top
    for i, ((label, bold, bars), h) in enumerate(zip(rows, heights)):
        if i % 2:
            grid.insert(0, f'<rect x="{PAD}" y="{row_y}" width="{WIDTH - 2 * PAD}" height="{h}" class="z"/>')
        if bold and i:
            grid.append(f'<line x1="{PAD}" y1="{row_y}" x2="{WIDTH - PAD}" y2="{row_y}" class="r"/>')
        weight, strong = (" b", ' class="b"') if bold else ("", "")
        labels.append(f'<text x="{n(x0 - 12)}" y="{n(row_y + h / 2 + 4.5)}" font-size="13" text-anchor="end"{strong}>{esc(label)}</text>')
        by = row_y + 8
        for platform, value, low, high, said, cargo in bars:
            if value is None:
                x = scale(base)
                marks.append(f'<rect x="{n(x + 1)}" y="{n(by + 1.5)}" width="9" height="9" rx="2" class="o w{platform}"/>')
                labels.append(f'<text x="{n(x + 16)}" y="{n(by + BAR - 2)}" font-size="11.5" class="d">{esc(said)}</text>')
                by += BAR + GAP
                continue
            a, b = sorted((scale(base), scale(min(max(value, lo), hi))))
            marks.append(f'<rect x="{n(a)}" y="{n(by)}" width="{n(max(b - a, 2))}" height="{BAR}" rx="2" class="p{platform}"/>')
            right = value >= base
            end = b if right else a
            if low is not None and high is not None:
                l, h_ = scale(max(low, lo)), scale(min(high, hi))
                mid = n(by + BAR / 2)
                marks.append(f'<path d="M{n(l)} {mid}H{n(h_)}M{n(l)} {n(by + 2)}v8M{n(h_)} {n(by + 2)}v8" class="o w{platform}"/>')
                end = max(end, h_) if right else min(end, l)
            if cargo is not None and lo < cargo < hi:
                marks.append(f'<rect x="{n(scale(cargo) - 1.25)}" y="{n(by - 2)}" width="2.5" height="{BAR + 4}" class="c"/>')
            anchor, tx = ("start", end + 6) if right else ("end", end - 6)
            labels.append(f'<text x="{n(tx)}" y="{n(by + BAR - 2)}" font-size="12" text-anchor="{anchor}" class="v{weight}">{esc(said)}</text>')
            by += BAR + GAP
        row_y += h
    if reference:
        x = scale(reference)
        marks.append(f'<line x1="{n(x)}" y1="{top - 8}" x2="{n(x)}" y2="{bottom}" class="r"/>')
        if reference_label:
            labels.append(f'<text x="{n(x)}" y="{top - 13}" font-size="11.5" text-anchor="middle" class="d">{esc(reference_label)}</text>')
        if hints:
            labels.append(f'<text x="{n(x - 8)}" y="{bottom + 37}" font-size="12" text-anchor="end" class="d">← {esc(hints[0])}</text>')
            labels.append(f'<text x="{n(x + 8)}" y="{bottom + 37}" font-size="12" class="d">{esc(hints[1])} →</text>')
    if axis_label:
        labels.append(f'<text x="{n((x0 + x1) / 2)}" y="{bottom + 37}" font-size="12" text-anchor="middle" class="d">{esc(axis_label)}</text>')
    y = bottom + 44
    for line in wrap(notes, 11.5, WIDTH - 2 * PAD):
        y += 17
        labels.append(f'<text x="{PAD}" y="{y}" font-size="11.5" class="d">{esc(line)}</text>')
    height = y + 14

    shapes = "\n".join(body + grid + marks + labels)
    return {scheme: (f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" '
                     f'role="img" aria-label="{esc(title)}">\n<title>{esc(title)}</title>\n{style(c)}\n{shapes}\n</svg>\n')
            for scheme, c in SCHEMES.items()}


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
    return f"{v:.0f}×" if v >= 9.95 else f"{v:.1f}×"


def gigabytes(mb):
    return f"{mb / 1000:.1f} GB" if mb >= 1000 else f"{mb:.0f} MB"


def median(values):
    r = sorted(values)
    return r[len(r) // 2] if len(r) % 2 else math.sqrt(r[len(r) // 2 - 1] * r[len(r) // 2])


def platforms(measured):
    return [name for name in NAMES if name in measured]


NOT_MEASURED = "not measured yet"
MACHINES = "Each colour is a machine of its own: compare bars of one colour, not across colours."


def edits(rows, where):
    """`rows`: (project, {platform: [cargo seconds / ra-jit seconds of each kind of edit]}).
    `where` names each platform's machine. The chart's alt text is its own summary."""
    rows = [(p, {k: sorted(v) for k, v in by.items() if v}) for p, by in rows]
    rows = [(p, by) for p, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    lo, hi = bounds([x for _, by in rows for r in by.values() for x in r], floor=1)
    bars = [(p, False, [(NAMES.index(k), median(by[k]), by[k][0], by[k][-1], times(median(by[k])), None) if k in by else
                        (NAMES.index(k), None, None, None, NOT_MEASURED, None)
                        for k in NAMES if k in measured]) for p, by in rows]
    write("edits", chart("Edit, then build: how many times faster ra-jit is than cargo",
                         "A small edit to a real project, then an incremental build: cargo build's time ÷ cargo ra-jit build's, "
                         "both warm. The bar is the median over the kinds of edit, the thin line runs from the slowest kind "
                         "to the fastest. Further right is better.",
                         bars, lo, hi, lambda v: f"{v:g}×", notes=f"{MACHINES} {where}", legend=platforms(measured),
                         reference=1.0, reference_label="as fast as cargo", hints=("ra-jit slower", "ra-jit faster")))
    return "; ".join(f"{p}: " + ", ".join(f"{k} {times(median(r))}" for k, r in by.items()) for p, by in rows)


def memory(rows, where):
    """`rows`: (project, {platform: (cargo's peak MB, ra-jit's peak MB)}) during an edit."""
    rows = [(p, {k: v for k, v in by.items() if v[0] and v[1]}) for p, by in rows]
    rows = [(p, by) for p, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    lo, hi = bounds([v for _, by in rows for pair in by.values() for v in pair], pad=1.6)
    bars = [(p, False, [(NAMES.index(k), by[k][1], None, None, f"{gigabytes(by[k][1])} · cargo {gigabytes(by[k][0])}", by[k][0])
                        if k in by else (NAMES.index(k), None, None, None, NOT_MEASURED, None)
                        for k in NAMES if k in measured]) for p, by in rows]
    write("memory", chart("Memory while an edit to one function builds",
                          "The most memory every process doing the work held at once. The bar is ra-jit's: its daemon, which "
                          "keeps the project loaded between edits, its workers and the command. The tick is plain cargo's: "
                          "cargo, rustc and the linker. Shorter is less memory.",
                          bars, lo, hi, lambda v: gigabytes(v).replace(".0 ", " "), notes=f"{MACHINES} {where}",
                          legend=platforms(measured), cargo_key="cargo's peak", axis_label="peak memory (log scale)"))
    return "; ".join(f"{p}: " + ", ".join(f"{k} {gigabytes(b)} for ra-jit, {gigabytes(a)} for cargo" for k, (a, b) in by.items())
                     for p, by in rows)


def runtime(rows, geomeans, where, rough=(), rough_note=""):
    """`rows`: (benchmark, {platform: ra-jit's time / release's}); `geomeans`: {platform: geometric mean};
    `rough`: the (benchmark, platform) whose runs varied too much to read closely, labelled ≈."""
    rows = [(b, {k: v for k, v in by.items() if v}) for b, by in rows]
    rows = [(b, by) for b, by in rows if by]
    if not rows:
        return None
    measured = {k for _, by in rows for k in by}
    values = [v for _, by in rows for v in by.values()] + [v for v in geomeans.values() if v]
    lo, hi = bounds(values + [1.0], pad=1.3)
    cells = lambda b, by: [(NAMES.index(k), by[k], None, None, ("≈" if (b, k) in rough else "") + f"{by[k]:.2f}×", None) if by.get(k) else
                           (NAMES.index(k), None, None, None, NOT_MEASURED, None) for k in NAMES if k in measured]
    bars = [(b, False, cells(b, by)) for b, by in rows]
    if any(geomeans.values()):
        bars.append(("Geometric mean", True, cells(None, geomeans)))
    marked = any(k in measured for _, k in rough)
    write("runtime", chart("How fast the programs ra-jit builds run",
                           "Each benchmark's time with ra-jit's LLVM build ÷ its time with cargo build --release, on the same "
                           "machine. 1× is as fast as release; further left is faster.",
                           bars, lo, hi, lambda v: f"{v:g}×", notes=f"{MACHINES} {where}" + (f" {rough_note}" if marked else ""),
                           legend=platforms(measured), reference=1.0, reference_label="as fast as release",
                           hints=("faster than release", "slower than release")))
    return "; ".join(f"{k} geometric mean {v:.2f}×" for k, v in geomeans.items() if v) or \
        "; ".join(f"{b}: " + ", ".join(f"{k} {v:.2f}×" for k, v in by.items()) for b, by in rows)
