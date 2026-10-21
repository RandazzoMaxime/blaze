#!/usr/bin/env python3
"""Scratch experiment (branch diag-perf-static-1021, not for main): how often perf, unwinding by
.eh_frame, puts a frame in data, for rustc's corpus binary and ra-jit's, and the stacks it does it in."""
import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import debuginfo as d  # noqa: E402
from difftest import Fixture  # noqa: E402

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)
fx = Fixture(os.path.join(d.HERE, "corpus"), ["cases"], ["-p", "runner"])
fx.rustc_build()
fx.ra_jit_build()
perf = shutil.which("perf")
again = 'for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do "$0" >/dev/null || exit; done'
for who, binary in (("rustc", fx.reference), ("ra-jit", fx.binary)):
    shutil.copy(binary, os.path.join(OUT, f"{who}.bin"))
    layout = subprocess.run(["readelf", "-lW", binary], capture_output=True, text=True).stdout
    open(os.path.join(OUT, f"{who}.layout"), "w").write(layout)
    listed = subprocess.run(["nm", "-C", "--defined-only", binary], capture_output=True, text=True).stdout
    data = {re.sub(r"::h[0-9a-f]{16}$", "", p[2]) for p in (l.split(None, 2) for l in listed.splitlines())
            if len(p) == 3 and p[1] in "bBdDrRvV"}
    print(f"{who}: libdw_shift {d.libdw_shift(binary):#x}, {len(data)} data symbols")
    total = bad = 0
    shown = []
    for i in range(30):
        rec = os.path.join(tempfile.mkdtemp(), "perf.data")
        subprocess.run([perf, "record", "-q", "-o", rec, "-F", "5000", "--call-graph", "dwarf", "--", "sh", "-c", again, binary],
                       cwd=fx.ws, capture_output=True, check=True)
        script = subprocess.run([perf, "script", "-i", rec, "-F", "ip,sym,symoff,dso,srcline"], capture_output=True, text=True).stdout
        stacks = [s for s in script.split("\n\n") if s.strip()]
        total += len(stacks)
        for stack in stacks:
            syms = re.findall(r"^\s+[0-9a-f]+ (\S+?)(?:\+0x[0-9a-f]+)? \(", stack, re.M)
            if any(s in data for s in syms):
                bad += 1
                shown.append(stack)
    print(f"{who}: {bad} of {total} stacks have a frame in data")
    open(os.path.join(OUT, f"{who}.bad-stacks"), "w").write("\n\n".join(shown))
    for stack in shown[:6]:
        print("\n".join("    " + it.strip()[:200] for it in stack.strip().splitlines()) + "\n    --")
