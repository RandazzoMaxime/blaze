#!/usr/bin/env python3
"""A build of two programs where cargo builds one, while the other runs and takes edits (macOS
on AArch64, Linux on x86-64): the running one is not linked again, so it is never told to
restart.

usage: live_fallback.py     (RA_JIT_BIN names the ra-jit built; cargo-ra-jit and ra-jit-rustc
                             are found next to it)

harness/live-fallback is a workspace of `shared`, `probe`, a program that polls ra-jit-live and
prints `shared::answer() + own()`, and `stale`, whose `main` the daemon leaves to rustc and which
calls rustc's copy of a function that calls `shared::answer`. Each build is of both. Once the
daemon serves it and `probe` runs:
  shared    an edit of `shared::answer` is a patch `probe` applies, and has cargo build `stale`
            (rustc's copy of what it calls is of before the edit): `probe` is not told to
            restart, and `stale` prints the new answer
  own       an edit of `probe`'s own code after that is a patch too, with no restart

Python 3.13, stdlib only. Every step has a timeout; a failure names the step.
"""

import asyncio
import os
import shutil
import sys
import tempfile
from pathlib import Path

from live_e2e import APPLY_TIMEOUT, ROOT, HERE, Running, StepError, build, serving, show_daemon_log, start

# How long a program told to restart takes to hear it: ra-jit-live looks at its executable a
# few times a second, and the daemon says so as it writes it.
RESTART_HEARD = 5


def edit(source: Path, function: str, old: int, new: int) -> None:
    text = source.read_text()
    needle = f"fn {function}() -> u64 {{\n    {old}\n}}"
    if needle not in text:
        raise StepError(f"edit: `{needle}` is not in {source}")
    source.write_text(text.replace(needle, f"fn {function}() -> u64 {{\n    {new}\n}}"))


async def applied(probe: Running, step: str, answer: int, since: int) -> None:
    """The edit arrives as a patch, the new answer runs, and no restart follows."""
    outcome = await probe.wait_for(f"{step}: patch applied", "outcome:", since)
    if "applied" not in outcome:
        raise StepError(f"{step}: the program did not take the edit as a patch: {outcome}")
    await probe.wait_for(f"{step}: the new code runs", f"answer {answer}", since)
    try:
        async with asyncio.timeout(RESTART_HEARD):
            while True:
                line = await probe.lines.get()
                probe.log.append(line)
                if line.startswith("outcome:"):
                    raise StepError(f"{step}: the program was told more after the patch: {line}")
    except TimeoutError:
        pass


async def main() -> None:
    bin_dir = Path(os.environ["RA_JIT_BIN"]).resolve().parent
    work = Path(tempfile.mkdtemp(prefix="ra-jit-live-fallback-"))
    probe: Running | None = None
    try:
        shutil.copytree(ROOT / "crates" / "ra-jit-live", work / "crates" / "ra-jit-live")
        shutil.copytree(HERE / "live-fallback", work / "harness" / "live-fallback", ignore=shutil.ignore_patterns("target"))
        project = work / "harness" / "live-fallback"
        target = work / "target"
        env = dict(
            os.environ,
            PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            CARGO_TARGET_DIR=str(target),
            RUST_BACKTRACE="0",
        )
        env.pop("CARGO_INCREMENTAL", None)

        await build("first build", project, env)
        await serving(project, env)
        print("daemon is serving builds")
        # The first edit after a daemon starts writes the whole program again.
        edit(project / "probe" / "src" / "main.rs", "own", 0, 10)
        await build("edit before start", project, env)
        probe = await start(target / "debug" / "probe", project, env, "start", 11)
        print("probe started with answer 11")

        edit(project / "shared" / "src" / "lib.rs", "answer", 1, 2)
        since = len(probe.log)
        said = await build("shared", project, env)
        if "building with cargo" not in said:
            raise StepError(f"shared: cargo was to build `stale`, and the build said:\n{said}")
        await applied(probe, "shared", 12, since)
        stale = await asyncio.create_subprocess_exec(str(target / "debug" / "stale"), stdout=asyncio.subprocess.PIPE)
        async with asyncio.timeout(APPLY_TIMEOUT):
            out, _ = await stale.communicate()
        if out.decode().strip() != "stale 2":
            raise StepError(f"shared: cargo's `stale` printed {out.decode().strip()!r}, not 'stale 2'")
        print("an edit cargo built `stale` for was a patch of the running probe, with no restart")

        edit(project / "probe" / "src" / "main.rs", "own", 10, 20)
        since = len(probe.log)
        await build("own", project, env)
        await applied(probe, "own", 22, since)
        print("the next edit of the probe's own code was a patch too")
    finally:
        if probe is not None:
            await probe.stop()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except StepError as error:
        print(f"error: {error}", file=sys.stderr)
        show_daemon_log()
        sys.exit(1)
