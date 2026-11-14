#!/usr/bin/env python3
"""Status line and final report for scripts/overnight.sh (reads its run directory).

usage: overnight_report.py --status <dir> <start> <end>
       overnight_report.py --final <dir> <start> <end> --deadline <s> --max-rss <MiB>

--final prints the markdown report and exits 1 when the run failed.
"""
import argparse
import json
import statistics
import sys
import time
from collections import defaultdict
from pathlib import Path

GATED = {"clean", "delay", "lossy"}  # any read error fails
LABELS = ["seq-read-1", "seq-read-16", "rand-read-4k", "rand-read-128k"]


def rows(path, n=None):
    if not path.exists():
        return []
    out = []
    for line in path.read_text().splitlines():
        f = line.split()
        if f and (n is None or len(f) >= n):
            out.append(f)
    return out


def load(d):
    passes = [
        dict(phase=f[0], cycle=int(f[1]), start=int(f[2]), end=int(f[3]),
             wrong=int(f[4]), ioerr=int(f[5]), listing=f[6] == "1",
             cut=len(f) > 7 and f[7] == "1")
        for f in rows(d / "passes", 7)
    ]
    samples = [
        dict(t=int(f[0]), phase=f[1], cycle=int(f[2]), client=f[3], rss=int(f[4]) / 1024,
             hwm=int(f[5]) / 1024, threads=int(f[6]), ticks=int(f[7]), fds=int(f[8]))
        for f in rows(d / "samples.tsv", 9)
    ]
    faults = [(int(f[0]), int(f[1])) for f in rows(d / "faults", 2)]
    perf = []
    if (d / "perf.jsonl").exists():
        perf = [json.loads(l) for l in (d / "perf.jsonl").read_text().splitlines() if l.strip()]
    phase_rss = [(f[0], int(f[1]), int(f[2]) / 1024) for f in rows(d / "phase_rss", 3)]
    return passes, samples, faults, perf, phase_rss


def hm(seconds):
    seconds = int(seconds)
    return f"{seconds // 3600}h{seconds % 3600 // 60:02d}m"


def status(d, start, end):
    passes, samples, _, perf, _ = load(d)
    now = time.time()
    phase = "-"
    if (d / "events").exists():
        for line in (d / "events").read_text().splitlines():
            f = line.split()
            if len(f) > 2 and f[1] == "phase":
                phase = " ".join(f[2:])
    rs = [s for s in samples if s["client"] == "rustern"]
    last = rs[-1] if rs else None
    print(f"elapsed {hm(now - start)} of {hm(end - start)}, phase {phase}")
    print(f"checksum passes {len(passes)}, wrong {sum(p['wrong'] for p in passes)}, "
          f"read errors {sum(p['ioerr'] for p in passes)}, "
          f"bad listings {sum(not p['listing'] for p in passes)}, "
          f"incomplete {sum(p['cut'] for p in passes)}")
    if last:
        print(f"rustern RSS {last['rss']:.0f} MiB (peak {max(s['hwm'] for s in rs):.0f}), "
              f"threads {last['threads']}, fds {last['fds']}")
    for flag in ["exited", "enotconn", "dead"]:
        if (d / flag).exists():
            print(f"!! {flag}")


def overlaps(p, faults, deadline):
    return any(s <= p["end"] and e + deadline >= p["start"] for s, e in faults)


def final(d, start, end, deadline, max_rss):
    passes, samples, faults, perf, phase_rss = load(d)
    fail = []
    for flag, why in [("exited", "rustern exited"), ("enotconn", "ENOTCONN seen"),
                      ("dead", "mount not alive at the end")]:
        if (d / flag).exists():
            fail.append(why)
    harsh_errors = 0
    for p in passes:
        where = f"{p['phase']} cycle {p['cycle']} at {time.strftime('%H:%M:%S', time.localtime(p['start']))}"
        if p["wrong"]:
            fail.append(f"{where}: {p['wrong']} wrong checksums")
        if not p["listing"]:
            fail.append(f"{where}: corpus listing differs")
        if p["cut"] and p["phase"] == "clean":
            fail.append(f"{where}: pass did not finish ({p['end'] - p['start']} s)")
        if p["ioerr"]:
            if p["phase"] in GATED:
                fail.append(f"{where}: {p['ioerr']} read errors")
            elif p["phase"] == "stall" and not overlaps(p, faults, deadline):
                fail.append(f"{where}: {p['ioerr']} read errors outside fault windows")
            elif p["phase"] == "harsh":
                harsh_errors += p["ioerr"]
    if not any(not p["cut"] for p in passes):
        fail.append("no checksum pass completed")
    rs = [s for s in samples if s["client"] == "rustern"]
    peak = max((s["hwm"] for s in rs), default=0)
    if peak > max_rss:
        fail.append(f"peak RSS {peak:.0f} MiB > {max_rss} MiB")
    perf_errors = [p for p in perf if p.get("errors") and p["client"] == "rustern"
                   and p["phase"] in GATED]
    for p in perf_errors:
        fail.append(f"fio {p['label']} failed on rustern in {p['phase']} cycle {p['cycle']}")

    result = "FAIL" if fail else "PASS"
    out = [f"## Overnight ({time.strftime('%Y-%m-%d', time.localtime(start))}, "
           f"{hm(end - start)}, loopback shaped with tc netem): {result}", ""]
    cycles = max((p["cycle"] for p in passes), default=0)
    cut = defaultdict(int)
    for p in passes:
        cut[p["phase"]] += p["cut"]
    cut_s = ", ".join(f"{n} in {ph}" for ph, n in cut.items() if n) or "none"
    out.append(f"- {cycles} cycles of phases; {len(passes)} checksum passes "
               f"(incomplete: {cut_s}; {sum(p['wrong'] for p in passes)} wrong, "
               f"{sum(not p['listing'] for p in passes)} bad listings, "
               f"{sum(p['ioerr'] for p in passes)} read errors, {harsh_errors} of them in "
               f"harsh phases); {len(faults)} SIGSTOP fault windows.")
    clean = [r for ph, _, r in phase_rss if ph == "clean"]
    if rs:
        cpu = (rs[-1]["ticks"] - rs[0]["ticks"]) / 100
        at_clean = ""
        if clean:
            at_clean = ("; RSS at the end of each clean phase: "
                        + ", ".join(f"{r:.0f}" for r in clean) + " MiB"
                        + (f" (growth {clean[-1] - clean[0]:+.0f} MiB)" if len(clean) > 1 else ""))
        out.append(f"- Rustern peak RSS {peak:.0f} MiB{at_clean}; threads {max(s['threads'] for s in rs)} max, fds "
                   f"{max(s['fds'] for s in rs)} max; CPU {cpu:.0f} s.")
    ts = [s for s in samples if s["client"] == "ternfuse"]
    if ts:
        out.append(f"- ternfuse peak RSS {max(s['hwm'] for s in ts):.0f} MiB.")
    for f in fail[:30]:
        out.append(f"- FAIL: {f}")
    if len(fail) > 30:
        out.append(f"- ... and {len(fail) - 30} more failures")

    # Medians over cycles, per phase and job; Rustern / ternfuse ratio where both ran.
    med = defaultdict(list)
    for p in perf:
        if not p.get("errors"):
            med[(p["phase"], p["label"], p["client"])].append(p)
    phases = list(dict.fromkeys(p["phase"] for p in perf))
    if phases:
        out += ["", "| Phase | Job | Rustern MiB/s | p99 µs | ternfuse MiB/s | p99 µs | Rustern / ternfuse |",
                "|---|---|---|---|---|---|---|"]
        for ph in phases:
            for label in LABELS:
                r = med.get((ph, label, "rustern"), [])
                t = med.get((ph, label, "ternfuse"), [])
                if not r and not t:
                    continue
                def cell(xs, k):
                    return f"{statistics.median(x[k] for x in xs):,.0f}" if xs else "-"
                ratio = "-"
                if r and t:
                    ratio = (f"{statistics.median(x['mib_s'] for x in r) / statistics.median(x['mib_s'] for x in t):.2f}")
                out.append(f"| {ph} | {label} | {cell(r, 'mib_s')} | {cell(r, 'p99_us')} | "
                           f"{cell(t, 'mib_s')} | {cell(t, 'p99_us')} | {ratio} |")
    print("\n".join(out) + "\n")
    return 1 if fail else 0


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--status", action="store_true")
    a.add_argument("--final", action="store_true")
    a.add_argument("dir", type=Path)
    a.add_argument("start", type=int)
    a.add_argument("end", type=int)
    a.add_argument("--deadline", type=int, default=60)
    a.add_argument("--max-rss", type=int, default=2048)
    args = a.parse_args()
    if args.final:
        sys.exit(final(args.dir, args.start, args.end, args.deadline, args.max_rss))
    status(args.dir, args.start, args.end)


if __name__ == "__main__":
    main()
