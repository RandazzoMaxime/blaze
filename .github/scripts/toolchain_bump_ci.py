#!/usr/bin/env python3
"""toolchain_bump_ci.py: the daily toolchain bump's steps (.github/workflows/toolchain-bump.yaml,
docs/toolchain.md "The daily bump"). blaze tracks the latest Rust nightly: each day the newest
nightly with every component blaze needs is pinned on the branch BRANCH, its pull request runs
the merge gate and the release train, and the train's verdict queues it or says what broke.

    prepare [--date D]      on BRANCH (the open bump's, with main merged in, or main's): run
                            harness/toolchain_bump.py, commit, push; outputs changed, pin
    answers FILE...         merge answers.py --write's rustc.json of each host into the tree's
    pr                      commit the answers, push, open or update the pull request, start
                            the train on BRANCH
    verdict RUN STD.json    the train run RUN of BRANCH and the std count: no job or harness
                            red that is green on main's newest finished train, and the std unit
                            refusing no more than main's: queue the pull request; else comment
                            and open or update the issue
    blocked FILE [--date D] open or update the issue with what FILE says failed

git pushes with the checkout's token, RA_JIT_BOT_TOKEN, since a push with GITHUB_TOKEN starts no
workflow; gh (GH_TOKEN) is GITHUB_TOKEN, which may open and comment on pull requests and issues
where the bot's token may not. A pull request GITHUB_TOKEN opens starts no workflow either, so it
is opened before the bot pushes the answers, whose push runs the gate on it. Python 3.11+,
stdlib, git and the gh CLI.
"""
import argparse, datetime, json, os, re, subprocess, sys

REPO = os.environ.get("GITHUB_REPOSITORY", "indexable-inc/blaze")
SERVER = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
RUN_URL = f"{SERVER}/{REPO}/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BRANCH = "toolchain/bump"
PIN = os.path.join(ROOT, "harness/toolchain")
PINS = os.path.join(ROOT, "crates/ra-jit/src/cargo/toolchain/pins.txt")
ANSWERS = os.path.join(ROOT, "crates/ra-jit/src/answers/rustc.json")
# The issue is one for as long as the bump is blocked, retitled to the nightly it is on.
TITLE = "Toolchain bump to {date} is blocked"
TITLED = re.compile(r"^Toolchain bump to (\d{4}-\d{2}-\d{2}) is blocked$")
# train.yaml's jobs that gather the others, and those that do not block it (train_issue.py).
GATHERERS = {"train", "train issue", "release"}
NOT_BLOCKING = ("nix / ", "linux x64 / bench")
FAILED = {"failure", "cancelled", "timed_out"}
ANSI = re.compile(r"\x1b\[[0-9;]*m")


class Failed(Exception):
    pass


def run(*argv, check=True, cwd=ROOT, env=None):
    done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, env=env)
    if check and done.returncode:
        raise Failed(f"`{' '.join(argv[:4])}` exited {done.returncode}:\n{(done.stderr or done.stdout)[-3000:]}")
    return done.stdout


def gh(*args, check=True):
    return run("gh", *args, check=check)


def output(**values):
    path = os.environ.get("GITHUB_OUTPUT")
    for key, value in values.items():
        print(f"{key}={value}")
        if path:
            open(path, "a").write(f"{key}={value}\n")


def pin():
    return open(PIN).read().strip()


def pins():
    lines = dict(l.split(" ", 1) for l in open(PINS).read().splitlines() if l and not l.startswith("#"))
    return lines["rustc"], lines["commit"]


def open_pr():
    found = json.loads(gh("pr", "list", "-R", REPO, "--head", BRANCH, "--state", "open",
                          "--json", "number,title,createdAt,url"))
    return found[0] if found else None


def days_behind(name, newest=None):
    date = datetime.date.fromisoformat(name.removeprefix("nightly-"))
    return ((newest or datetime.date.today()) - date).days


# ---------------------------------------------------------------------------------------------
# The issue


def find_issue():
    listed = json.loads(gh("issue", "list", "-R", REPO, "--state", "open", "--search",
                           '"Toolchain bump to" in:title', "--json", "number,title"))
    return next((it["number"] for it in listed if TITLED.match(it["title"])), None)


def blocked(date, body):
    """Opens the issue for the bump to `date`, or retitles and comments on the open one."""
    title = TITLE.format(date=date)
    main_pin = run("git", "show", "origin/main:harness/toolchain", check=False).strip() or pin()
    body += (f"\n\nmain is pinned to {main_pin}, {days_behind(main_pin)} days behind today. "
             f"The bump retries every day on top of main (docs/toolchain.md, \"The daily bump\"). Run: {RUN_URL}")
    number = find_issue()
    if number:
        gh("issue", "edit", str(number), "-R", REPO, "--title", title)
        gh("issue", "comment", str(number), "-R", REPO, "--body", body)
        print(f"#{number}: commented\n{body}")
    else:
        url = gh("issue", "create", "-R", REPO, "--title", title, "--body", body).strip()
        print(f"{url}: opened\n{body}")


def unblocked(date, why):
    number = find_issue()
    if number:
        gh("issue", "close", str(number), "-R", REPO, "--comment", f"The bump to {date} {why}: {RUN_URL}")
        print(f"#{number}: closed")


# ---------------------------------------------------------------------------------------------
# prepare


def prepare(date):
    run("git", "fetch", "-q", "origin", "main")
    pr = open_pr()
    if pr:
        run("git", "fetch", "-q", "origin", BRANCH)
        run("git", "checkout", "-q", "-B", BRANCH, f"origin/{BRANCH}")
        merged = run("git", "merge", "--no-edit", "-q", "origin/main", check=False)
        if run("git", "diff", "--name-only", "--diff-filter=U").strip():
            files = run("git", "diff", "--name-only", "--diff-filter=U").split()
            run("git", "merge", "--abort", check=False)
            raise Failed(f"merging main into the open bump {pr['url']} conflicts in: {', '.join(files)}. "
                         f"Resolve it on {BRANCH} by hand.{merged}")
        base = f"origin/{BRANCH}"
        # A bump open for a day without merging is visible, whatever its train said.
        opened = datetime.datetime.fromisoformat(pr["createdAt"].replace("Z", "+00:00"))
        if datetime.datetime.now(datetime.timezone.utc) - opened > datetime.timedelta(hours=20):
            blocked(pin().removeprefix("nightly-"),
                    f"The bump {pr['url']} ({pr['title']}) has been open since {pr['createdAt']} without "
                    "merging. Its checks and the train's verdict on it say why.")
    else:
        run("git", "checkout", "-q", "-B", BRANCH, "origin/main")
        base = "origin/main"
    was = pin()
    argv = [sys.executable, "harness/toolchain_bump.py", *(["--date", date] if date else [])]
    bumped = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True)
    print(bumped.stdout, bumped.stderr, sep="\n")
    if bumped.returncode:
        raise Failed(f"`{' '.join(argv[1:])}` failed, moving {was}:\n\n```\n{(bumped.stderr or bumped.stdout)[-4000:]}\n```")
    now = pin()
    run("git", "add", "-A")
    if run("git", "status", "--porcelain").strip():
        rustc, commit = pins()
        run("git", "commit", "-q", "-m", f"toolchain: pin {now}\n\nrustc {rustc}, rust-lang/rust {commit}.\n"
            "harness/toolchain_bump.py, run by .github/workflows/toolchain-bump.yaml.")
    changed = run("git", "rev-parse", "HEAD").strip() != run("git", "rev-parse", base).strip()
    if changed:
        # With no bump open the branch is the workflow's alone, left by a run that stopped.
        run("git", "push", "-q", *([] if pr else ["--force"]), "origin", f"HEAD:refs/heads/{BRANCH}")
    output(changed=str(changed).lower(), pin=now, was=was)


# ---------------------------------------------------------------------------------------------
# answers


def answers(files):
    """One rustc.json from each host's: the same answers but for each host's target specs."""
    merged = None
    for path in files:
        data = json.load(open(path))
        if merged is None:
            merged = data
            continue
        if data["version"] != merged["version"]:
            raise Failed(f"{path} has the answers of rustc {data['version']}, the others of {merged['version']}")
        for target, its in data["targets"].items():
            mine = merged["targets"].setdefault(target, its)
            for key, value in its.items():
                if key == "target_spec_json":
                    mine[key].update(value)
                elif mine.get(key) != value:
                    raise Failed(f"{path}: {target}'s {key} differs from another host's")
    with open(ANSWERS, "w") as out:
        json.dump(merged, out, indent=1, sort_keys=True)
        out.write("\n")
    hosts = sorted({h for t in merged["targets"].values() for h in t["target_spec_json"]})
    print(f"answers of rustc {merged['version']}, target specs of {', '.join(hosts)}")


# ---------------------------------------------------------------------------------------------
# pr


def pr():
    name = pin()
    rustc, commit = pins()
    run("git", "add", ANSWERS)
    if run("git", "status", "--porcelain", ANSWERS).strip():
        run("git", "commit", "-q", "-m", f"toolchain: answers of {name} (answers.py --write on each CI host)")
    existing = open_pr()
    if not existing and run("git", "rev-parse", "HEAD").strip() == run("git", "rev-parse", f"origin/{BRANCH}").strip():
        run("git", "commit", "-q", "--allow-empty", "-m", f"toolchain: run the merge gate on {name}")
    main_pin = run("git", "show", "origin/main:harness/toolchain").strip()
    main_commit = re.search(r"(?m)^commit (\w+)", run("git", "show", f"origin/main:{os.path.relpath(PINS, ROOT)}")).group(1)
    title = f"toolchain: pin {name}"
    body = "\n".join([
        f"Moves blaze's toolchain from {main_pin} to {name} (rustc {rustc}).",
        "",
        f"- upstream changes: https://github.com/rust-lang/rust/compare/{main_commit[:12]}...{commit[:12]}",
        f"- opened by the daily bump ({RUN_URL}), `harness/toolchain_bump.py` and `answers.py --write` on "
        "Linux x86-64, Linux AArch64 and macOS arm64",
        "",
        "The merge gate runs on it as on any pull request; the release train runs on this branch, and "
        "when no job or harness is red that is green on main's newest finished train, and the standard "
        "library refuses no more functions than main's (`harness/stdcount.py`), it is queued with "
        "`gh pr merge --auto`. Otherwise the verdict is a comment here and the issue \"Toolchain bump to "
        "<date> is blocked\". Fixes for what the new nightly breaks go on this branch; the next daily run "
        "merges main into it and moves it to the newest nightly, keeping them (docs/toolchain.md).",
    ])
    if existing:
        gh("pr", "edit", str(existing["number"]), "-R", REPO, "--title", title, "--body", body)
        number = existing["number"]
    else:
        url = gh("pr", "create", "-R", REPO, "--base", "main", "--head", BRANCH, "--title", title, "--body", body).strip()
        number = int(url.rsplit("/", 1)[1])
    # The bot's push, after the pull request is open, is what runs the merge gate on it.
    run("git", "push", "-q", "origin", f"HEAD:refs/heads/{BRANCH}")
    output(number=number)
    gh("workflow", "run", "train.yaml", "-R", REPO, "--ref", BRANCH)
    print(f"#{number}: {title}; the train is started on {BRANCH}")


# ---------------------------------------------------------------------------------------------
# verdict


def jobs_of(run_id):
    out = gh("api", "--paginate", f"repos/{REPO}/actions/runs/{run_id}/jobs?per_page=100", "--jq", ".jobs[]")
    return [json.loads(l) for l in out.splitlines() if l.strip()]


def failed_harnesses(job_id):
    # A newer gh refuses to print a log with terminal escapes without the flag; an older one has
    # no such flag.
    path = f"repos/{REPO}/actions/jobs/{job_id}/logs"
    done = subprocess.run(["gh", "api", "--allow-escape-sequences", path], capture_output=True, text=True)
    if done.returncode and "unknown flag" in done.stderr:
        done = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    log = done.stdout
    found = re.findall(r"failed: (.+)$", ANSI.sub("", log), re.M)
    if not found or found[-1].strip() == "none":
        return None
    return {h.strip() for h in found[-1].split(",") if h.strip()}


def failures(run_id):
    """{job name: set of harnesses that failed, or None for the job as a whole} of a train run."""
    red = {}
    for job in jobs_of(run_id):
        if job["conclusion"] in FAILED and job["name"] not in GATHERERS and not job["name"].startswith(NOT_BLOCKING):
            red[job["name"]] = failed_harnesses(job["id"]) if "harness" in job["name"] else None
    return red


def main_train(workflow_id):
    """main's newest train that finished (not cancelled): its id and head commit."""
    runs = json.loads(gh("api", f"repos/{REPO}/actions/workflows/{workflow_id}/runs?branch=main&status=completed&per_page=50",
                         "--jq", "[.workflow_runs[] | select(.conclusion != \"cancelled\") | {id, head_sha, html_url}]"))
    return runs[0] if runs else None


def verdict(run_id, std_path):
    train = json.loads(gh("api", f"repos/{REPO}/actions/runs/{run_id}"))
    # The verdict runs main's checkout: the bump's pin is read at the train's commit.
    name = gh("api", f"repos/{REPO}/contents/harness/toolchain?ref={train['head_sha']}",
              "-H", "Accept: application/vnd.github.raw").strip()
    date = name.removeprefix("nightly-")
    pr = open_pr()
    if not pr:
        print(f"no open pull request from {BRANCH}; nothing to decide")
        return
    if train["conclusion"] == "cancelled":
        print("the train was cancelled: a newer one decides")
        return
    ours = failures(run_id)
    base = main_train(train["workflow_id"])
    theirs = failures(base["id"]) if base else {}
    new = []
    for job, harnesses in sorted(ours.items()):
        if job not in theirs:
            new.append(f"- {job}" + (f": {', '.join(sorted(harnesses))}" if harnesses else ""))
        elif harnesses and theirs[job] is not None and (harnesses - theirs[job]):
            new.append(f"- {job}: {', '.join(sorted(harnesses - theirs[job]))} (main's train has {', '.join(sorted(theirs[job]))} red)")
    already = [f"- {job}" for job in sorted(ours) if job in theirs and not any(l.startswith(f"- {job}") for l in new)]
    std = None
    if std_path and os.path.exists(std_path):
        std = json.load(open(std_path))
    std_line = "the std count did not run (its job failed: see the run)"
    std_worse = True
    if std and "base" in std:
        n, b = std["new"], std["base"]
        std_worse = n["refused"] > b["refused"]
        std_line = (f"the standard library: {name} refuses {n['refused']} of {n['total']} fns, main's "
                    f"{b['toolchain']} {b['refused']} of {b['total']} ({n['refused'] - b['refused']:+d})")
    lines = [f"Train on {train['head_sha'][:10]}: {train['html_url']}",
             f"Compared with main's newest finished train: {base['html_url'] if base else 'none found'}", "",
             f"- {std_line}"]
    if new:
        lines += ["", "Red here and green on main (what the bump broke):", *new]
    if already:
        lines += ["", "Red here and on main too (not counted against the bump):", *already]
    body = "\n".join(lines)
    if new or std_worse:
        gh("pr", "comment", str(pr["number"]), "-R", REPO, "--body", f"**The bump is blocked.**\n\n{body}")
        blocked(date, f"The bump {pr['url']} ({name}) is red.\n\n{body}")
        sys.exit(1)
    gh("pr", "merge", str(pr["number"]), "-R", REPO, "--merge", "--auto")
    gh("pr", "comment", str(pr["number"]), "-R", REPO,
       "--body", f"**The bump's gates are green**: queued with `gh pr merge --auto`.\n\n{body}")
    unblocked(date, f"is green and queued ({pr['url']})")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="step", required=True)
    sub.add_parser("prepare").add_argument("--date")
    sub.add_parser("answers").add_argument("files", nargs="+")
    sub.add_parser("pr")
    v = sub.add_parser("verdict")
    v.add_argument("run")
    v.add_argument("std", nargs="?")
    b = sub.add_parser("blocked")
    b.add_argument("why_file")
    b.add_argument("--date", default=datetime.date.today().isoformat())
    args = ap.parse_args()
    try:
        if args.step == "prepare":
            prepare(args.date)
        elif args.step == "answers":
            answers(args.files)
        elif args.step == "pr":
            pr()
        elif args.step == "verdict":
            verdict(args.run, args.std)
        else:
            why = open(args.why_file).read() if os.path.exists(args.why_file) else "a step failed before it said why"
            blocked(args.date, f"The daily bump stopped before its pull request:\n\n{why}")
    except Failed as error:
        print(f"error: toolchain_bump_ci {args.step}: {error}", file=sys.stderr)
        if args.step == "prepare":
            open(os.path.join(os.environ.get("RUNNER_TEMP", "/tmp"), "bump-failed.txt"), "w").write(str(error))
        sys.exit(1)


if __name__ == "__main__":
    main()
