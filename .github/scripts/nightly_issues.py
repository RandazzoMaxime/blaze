#!/usr/bin/env python3
"""nightly_issues.py: one issue per job that failed in this nightly run, opened or added to.

    python3 .github/scripts/nightly_issues.py

Run by .github/actions/nightly-issue in a workflow's last job, with GH_TOKEN (issues: write,
actions: read), GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT, GITHUB_WORKFLOW and
GITHUB_SHA set, as Actions sets them. Each job of the run that failed, timed out or was cancelled
gets the issue titled "Nightly <workflow>: <job>": a comment on it while it is open, a new one
otherwise, with the run, the commit and, for a harness part, the harnesses that failed (ci.py's
`failed:` line from the job's log). One issue per job, so that each failing check is a thing to
fix of its own, and a job that keeps failing keeps one issue. Jobs that only gather the others
(`ci`, `nightly issue`) are left out. Python 3.11+, stdlib and the `gh` CLI.
"""
import asyncio, json, os, re, sys

REPO = os.environ["GITHUB_REPOSITORY"]
RUN = os.environ["GITHUB_RUN_ID"]
ATTEMPT = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
WORKFLOW = os.environ["GITHUB_WORKFLOW"]
SHA = os.environ["GITHUB_SHA"][:10]
RUN_URL = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{REPO}/actions/runs/{RUN}"
FAILED = {"failure", "cancelled", "timed_out"}
GATHERERS = {"ci", "nightly issue"}
ANSI = re.compile(r"\x1b\[[0-9;]*m")


class GhFailed(Exception):
    pass


async def gh(*args, step):
    proc = await asyncio.create_subprocess_exec("gh", *args, stdout=asyncio.subprocess.PIPE,
                                                stderr=asyncio.subprocess.PIPE)
    async with asyncio.timeout(120):
        out, err = await proc.communicate()
    if proc.returncode:
        tail = err.decode(errors="replace").strip()[-800:]
        raise GhFailed(f"{step}: gh {' '.join(args[:3])} exited {proc.returncode}: {tail}")
    return out.decode(errors="replace")


async def failed_harnesses(job_id):
    """ci.py's `failed:` line of a harness job's log, or None for a job with none."""
    log = await gh("api", f"repos/{REPO}/actions/jobs/{job_id}/logs", step=f"the log of job {job_id}")
    found = re.findall(r"failed: (.+)$", ANSI.sub("", log), re.M)
    return found[-1].strip() if found and found[-1].strip() != "none" else None


async def report(job, open_issues):
    name = job["name"]
    title = f"Nightly {WORKFLOW}: {name}"
    lines = [f"[{name}]({job['html_url']}) {job['conclusion']} in {RUN_URL} at {SHA}."]
    if job["conclusion"] == "failure" and "harness" in name:
        harnesses = await failed_harnesses(job["id"])
        if harnesses:
            lines.append(f"\nThe harnesses that failed: {harnesses}.")
    body = "\n".join(lines)
    number = open_issues.get(title)
    if number:
        await gh("issue", "comment", str(number), "-R", REPO, "--body", body, step=f"a comment on #{number}")
        print(f"#{number} {title}: commented")
    else:
        url = await gh("issue", "create", "-R", REPO, "--title", title, "--body", body, step=f"the issue {title!r}")
        print(f"{url.strip()} {title}: opened")


async def main():
    jobs_json = await gh("api", "--paginate", f"repos/{REPO}/actions/runs/{RUN}/attempts/{ATTEMPT}/jobs?per_page=100",
                         "--jq", ".jobs[]", step="the run's jobs")
    jobs = [json.loads(line) for line in jobs_json.splitlines() if line.strip()]
    failed = [j for j in jobs if j["conclusion"] in FAILED and j["name"] not in GATHERERS]
    if not failed:
        print(f"no job of {RUN_URL} failed by name; nothing to report")
        return
    listed = await gh("issue", "list", "-R", REPO, "--state", "open", "--limit", "500",
                      "--search", f'"Nightly {WORKFLOW}:" in:title', "--json", "number,title", step="the open issues")
    open_issues = {it["title"]: it["number"] for it in json.loads(listed)}
    errors = []

    async def one(job):
        try:
            await report(job, open_issues)
        except (GhFailed, TimeoutError) as error:
            errors.append(f"{job['name']}: {error}")

    async with asyncio.TaskGroup() as group:
        for job in failed:
            group.create_task(one(job))
    if errors:
        sys.exit("nightly_issues.py: some failures were not reported:\n" + "\n".join(errors))


if __name__ == "__main__":
    asyncio.run(main())
