#!/usr/bin/env bash
# Benchmarks Rustern against the reference client on the same cluster (SC-005, spec:
# Performance Targets) and writes specs/001-fuse-client-poc/benchmarks.md.
#
# usage: scripts/bench.sh <ref-mount> [<rustern-mount>]
#
# <ref-mount> is a running ternfuse mount holding corpus/ (scripts/make-corpus.sh).
# <rustern-mount> (default: rustern-mnt next to <ref-mount>) is an empty directory: the
# script mounts Rustern there itself (twice: default and `--memory-limit 256MiB`) so it
# knows the daemon's pid. Keep it at the same path depth as <ref-mount>: `stat` of a cached
# file is a kernel path walk, and each extra component adds to it. For the memory-limited
# comparison it also mounts a fresh ternfuse (same binary as <ref-mount>'s) with
# GOMEMLIMIT=256MiB in ternfuse-limit-mnt next to <ref-mount>.
#
# All fio jobs use O_DIRECT so that readers of one file do not share the kernel page cache:
# every byte goes through the client under test. Most jobs read the 1 GiB corpus file; the
# `-file4G` jobs read bench/size_4294967296, created through <ref-mount> on first use
# (outside corpus/, so corpus.sha256 is unchanged). Results are recorded, not gated.
#
# Each fio job runs for a fixed time (sequential jobs loop over their file). Every job runs
# FIO_ROUNDS times and metadata latency LAT_ROUNDS times; in each round a job runs on both
# clients back to back, and which client goes first swaps every round, so a change in
# machine load hits both. The report gives the median over rounds and the range of the
# per-round Rustern / ternfuse ratio. With passwordless sudo, the host page cache (block
# services' files, shard databases) is dropped before every timed run, so each starts cold
# for both clients; dentries and inodes are dropped too, so the long-lived <ref-mount> and
# the fresh Rustern mount start each run with the same kernel caches. Without sudo, an untimed read of both files first warms that cache instead.
#
# Environment: RUSTERN_REGISTRY (127.0.0.1:10001), READERS ("1 2 4 8 16"),
# RUNTIME (rand-read seconds, 30), SEQ_RUNTIME (seq-read seconds, 10), FIO_ROUNDS (3),
# LAT_ROUNDS (5), LAT_N (meta_latency iterations per round, 10000), OUT (benchmarks.md path).
set -euo pipefail
cd "$(dirname "$0")/.."

ref=${1:?usage: bench.sh <ref-mount> [<rustern-mount>]}
mnt=${2:-$(dirname "$ref")/rustern-mnt}
tmnt=$(dirname "$ref")/ternfuse-limit-mnt
registry=${RUSTERN_REGISTRY:-127.0.0.1:10001}
readers=${READERS:-1 2 4 8 16}
runtime=${RUNTIME:-30}
seq_runtime=${SEQ_RUNTIME:-10}
fio_rounds=${FIO_ROUNDS:-3}
lat_rounds=${LAT_ROUNDS:-5}
lat_n=${LAT_N:-10000}
out=${OUT:-specs/001-fuse-client-poc/benchmarks.md}
big=corpus/sizes/size_1073741824
big4=bench/size_4294967296

need() { command -v "$1" >/dev/null || {
  echo "bench: missing required tool: $1" >&2
  exit 1
}; }
need fio
need python3
need fusermount3
need cargo
[[ -f $ref/$big ]] || {
  echo "bench: $ref/$big missing; run scripts/make-corpus.sh" >&2
  exit 1
}

ref_pid=$(pgrep -f "ternfuse.* $(realpath "$ref")\$" | head -1 || true)
[[ -n $ref_pid ]] || {
  echo "bench: no ternfuse process serving $ref" >&2
  exit 1
}
ref_bin=$(readlink "/proc/$ref_pid/exe")

if [[ ! -f $ref/$big4 ]]; then
  # Built on local disk (not a tmpfs) and copied in with cp: no redirects into the mount.
  echo "bench: creating $ref/$big4"
  mkdir -p target "$ref/bench"
  tmpd=$(mktemp -d target/bench-file.XXXXXX)
  head -c 4294967296 /dev/urandom >"$tmpd/${big4##*/}"
  cp -- "$tmpd/${big4##*/}" "$ref/$big4"
  rm -rf -- "$tmpd"
fi

cargo build --release -q -p rustern
cargo build --release -q -p rustern --example meta_latency
bin=target/release/rustern
latency=target/release/examples/meta_latency

work=$(mktemp -d "${TMPDIR:-/tmp}/rustern-bench.XXXXXX")
rustern_pid=
tf_pid=
# A FUSE unmount can fail with EBUSY for a moment after the last reader closed its files,
# while the kernel still sends their releases; retry for up to 5 s.
unmount() {
  for _ in $(seq 50); do
    fusermount3 -u "$1" 2>/dev/null && return
    sleep 0.1
  done
  fusermount3 -u "$1"
}
cleanup() {
  [[ -n $rustern_pid ]] && {
    kill -TERM "$rustern_pid" 2>/dev/null
    wait "$rustern_pid" 2>/dev/null
  }
  mountpoint -q "$mnt" && unmount "$mnt"
  mountpoint -q "$tmnt" && unmount "$tmnt"
  [[ -n $tf_pid ]] && {
    kill -TERM "$tf_pid" 2>/dev/null
    wait "$tf_pid" 2>/dev/null
  }
  rmdir -- "$mnt" "$tmnt" 2>/dev/null
  rm -rf -- "$work"
}
trap cleanup EXIT

mount_rustern() { # [extra flags...]
  mkdir -p "$mnt"
  "$bin" -registry "$registry" -log-file "$work/rustern.log" "$@" "$mnt" &
  rustern_pid=$!
  for _ in $(seq 100); do
    mountpoint -q "$mnt" && return
    sleep 0.1
  done
  echo "bench: rustern did not mount within 10 s" >&2
  exit 1
}
unmount_rustern() {
  kill -TERM "$rustern_pid"
  wait "$rustern_pid" || true
  rustern_pid=
}
mount_ternfuse() { # <GOMEMLIMIT>: a second, fresh ternfuse next to <ref-mount>
  mkdir -p "$tmnt"
  GOMEMLIMIT=$1 "$ref_bin" -log-file "$work/ternfuse.log" -registry "$registry" "$tmnt" &
  tf_pid=$!
  for _ in $(seq 100); do
    mountpoint -q "$tmnt" && return
    sleep 0.1
  done
  echo "bench: ternfuse did not mount within 10 s" >&2
  exit 1
}
unmount_ternfuse() {
  unmount "$tmnt"
  wait "$tf_pid" || true
  tf_pid=
}

cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
hwm_kib() { awk '/^VmHWM/ {print $2}' "/proc/$1/status"; }
rss_kib() { awk '/^VmRSS/ {print $2}' "/proc/$1/status"; }
# Resets the peak RSS (VmHWM) to the current RSS, so each workload's peak is its own.
reset_hwm() { echo 5 >"/proc/$1/clear_refs"; }
# Page cache, dentries and inodes (3): every timed run starts with cold kernel caches.
if sudo -n true 2>/dev/null; then cold=1; else cold=0; fi
drop_caches() {
  ((cold)) || return 0
  sync
  echo 3 | sudo -n tee /proc/sys/vm/drop_caches >/dev/null
}
rcvbuf_errors() { awk '/^Udp:/ {n++; if (n == 2) print $6}' /proc/net/snmp; }

# fio_run <label> <client> <pid> <round> <fio args...>: appends one JSON line to $work/results.
fio_run() {
  local label=$1 client=$2 pid=$3 round=$4
  shift 4
  local t0 t1 rss0
  drop_caches
  rss0=$(rss_kib "$pid")
  reset_hwm "$pid"
  t0=$(cpu_ticks "$pid")
  fio --output-format=json --readonly --direct=1 --invalidate=1 --ioengine=psync \
    --group_reporting --name="$label" "$@" >"$work/fio.json"
  t1=$(cpu_ticks "$pid")
  python3 - "$label" "$client" "$round" "$((t1 - t0))" "$(getconf CLK_TCK)" "$(hwm_kib "$pid")" \
    "$rss0" "$work/fio.json" >>"$work/results" <<'EOF'
import json, sys
label, client, rnd, ticks, hz, hwm, rss0, path = sys.argv[1:]
r = json.load(open(path))["jobs"][0]["read"]
gib = r["io_bytes"] / 2**30
p = r["clat_ns"]["percentile"]
print(json.dumps({"label": label, "client": client, "round": int(rnd), "mib_s": r["bw_bytes"] / 2**20,
  "iops": r["iops"], "p50_us": p["50.000000"] / 1e3, "p99_us": p["99.000000"] / 1e3,
  "cpu_s_per_gib": int(ticks) / int(hz) / gib if gib else 0, "hwm_mib": int(hwm) / 1024,
  "rss0_mib": int(rss0) / 1024}))
EOF
}

latency_run() { # <client> <mount> <round>
  drop_caches
  "$latency" "$2" "$lat_n" "$ref" | while read -r metric p50 p99 n; do
    printf '{"label": "%s", "client": "%s", "round": %s, "p50_us": %s, "p99_us": %s, "n": %s}\n' \
      "$metric" "$1" "$3" "$p50" "$p99" "$n"
  done >>"$work/results"
}

declare -A mnt_of pid_of
# Client order for round <r>: ternfuse first in odd rounds, Rustern first in even ones.
order() { if (($1 % 2)); then echo ternfuse rustern; else echo rustern ternfuse; fi; }

job() { # <round> <label> <file> <fio args...>: one fio job on both clients, back to back
  local r=$1 label=$2 f=$3 c
  shift 3
  for c in $(order "$r"); do
    echo "bench: round $r, $label, $c"
    fio_run "$label" "$c" "${pid_of[$c]}" "$r" --filename="${mnt_of[$c]}/$f" "$@"
  done
}

suite() { # <round>
  local r=$1 n bs
  for n in $readers; do
    job "$r" "seq-read-$n" "$big" --rw=read --bs=1M --size=1G --numjobs="$n" \
      --time_based --runtime="$seq_runtime"
  done
  for n in 1 16; do
    job "$r" "seq-read-$n-file4G" "$big4" --rw=read --bs=1M --size=4G --numjobs="$n" \
      --time_based --runtime="$seq_runtime"
  done
  for bs in 4k 128k; do
    job "$r" "rand-read-$bs" "$big" --rw=randread --bs="$bs" --size=1G --numjobs=16 \
      --time_based --runtime="$runtime"
  done
}

drops0=$(rcvbuf_errors)
mount_rustern
mnt_of=([ternfuse]=$ref [rustern]=$mnt)
pid_of=([ternfuse]=$ref_pid [rustern]=$rustern_pid)
if ((cold)); then
  echo "bench: dropping the page cache before every timed run"
else
  echo "bench: no passwordless sudo; warm-up read of $big and $big4 instead of dropping caches"
  for f in "$big" "$big4"; do
    fio --readonly --direct=1 --ioengine=psync --name=warm-up --filename="$ref/$f" --rw=read \
      --bs=1M --output=/dev/null
  done
fi
for r in $(seq "$fio_rounds"); do suite "$r"; done
for r in $(seq "$lat_rounds"); do
  echo "bench: round $r of $lat_rounds, meta-latency and readdir-latency"
  for c in $(order "$r"); do latency_run "$c" "${mnt_of[$c]}" "$r"; done
done
unmount_rustern
# Memory-limited, both mounted fresh side by side: Rustern's --memory-limit caps its read
# buffers; GOMEMLIMIT is the Go runtime's soft limit, the nearest ternfuse equivalent.
mount_ternfuse 256MiB
mount_rustern --memory-limit 256MiB
mnt_of=([ternfuse]=$tmnt [rustern]=$mnt)
pid_of=([ternfuse]=$tf_pid [rustern]=$rustern_pid)
for r in $(seq "$fio_rounds"); do
  job "$r" seq-read-16-limit256M "$big" --rw=read --bs=1M --size=1G --numjobs=16 \
    --time_based --runtime="$seq_runtime"
done
unmount_rustern
unmount_ternfuse
drops1=$(rcvbuf_errors)
# Per-round rows, kept for a look behind the medians (the work dir is removed on exit).
cp -- "$work/results" target/bench-results.jsonl

python3 - "$work/results" "$out" "$((drops1 - drops0))" "$(git -C ternfs rev-parse --short HEAD)" \
  "$(nproc)" "$(uname -r)" "$lat_n" "$seq_runtime" "$runtime" "$fio_rounds" "$lat_rounds" "$cold" <<'EOF'
import json, sys, datetime
path, out, drops, commit, cpus, kernel, lat_n, seq_runtime, runtime, fio_rounds, lat_rounds, cold = sys.argv[1:]
MARK = "<!-- end of generated results: notes below are kept by scripts/bench.sh -->"
import statistics
raw = [json.loads(l) for l in open(path)]
# Every result comes in rounds: keep one row per (label, client) with the median over
# rounds, plus the rounds themselves (round i of both clients ran back to back). Memory:
# the start of the first round and the highest peak of any round.
rows, rounds = [], {}
for r in raw:
    rounds.setdefault((r["label"], r["client"]), {})[r["round"]] = r
for (label, client), byr in rounds.items():
    xs = list(byr.values())
    row = {"label": label, "client": client, "rounds": byr}
    for k in ("mib_s", "iops", "p50_us", "p99_us", "cpu_s_per_gib"):
        if k in xs[0]:
            row[k] = statistics.median(x[k] for x in xs)
    if "hwm_mib" in xs[0]:
        row["hwm_mib"] = max(x["hwm_mib"] for x in xs)
        row["rss0_mib"] = byr[min(byr)]["rss0_mib"]
    rows.append(row)
get = lambda label, client: next((r for r in rows if r["label"] == label and r["client"] == client), None)
fio_label = lambda l: l.startswith(("seq-read", "rand-read"))
labels = list(dict.fromkeys(r["label"] for r in rows))
labels = [l for l in labels if fio_label(l)] + [l for l in labels if not fio_label(l)]

def spread(a, b, key):
    rs = [b["rounds"][i][key] / a["rounds"][i][key] for i in a["rounds"]
          if i in b["rounds"] and a["rounds"][i].get(key)] if a and b else []
    return f" ({min(rs):.2f}-{max(rs):.2f})" if len(rs) > 1 else ""

def ratio(a, b, key):
    return f"{b[key] / a[key]:.2f}" if a and b and a.get(key) else "–"

lines = [
  "# Benchmarks: Rustern vs ternfuse",
  "",
  f"Generated by `scripts/bench.sh` on {datetime.date.today()}: ternfs `{commit}`, "
  f"{cpus} CPUs, Linux {kernel}, local cluster. All fio jobs use O_DIRECT; they read the "
  "1 GiB corpus file, except `-file4G` jobs, which read a 4 GiB file; sequential jobs run "
  f"{seq_runtime} s (looping over the file), random ones {runtime} s. `-limit256M` jobs run "
  "on fresh mounts: Rustern with `--memory-limit 256MiB`, ternfuse with `GOMEMLIMIT=256MiB` "
  "(the Go runtime's soft limit), both mounted fresh side by side. Every fio row is the median "
  f"of {fio_rounds} rounds and every metadata row of {lat_rounds}; in each round both clients ran "
  "the job back to back, the first client swapping each round. Ranges in parentheses are the "
  "lowest and highest per-round Rustern / ternfuse ratio. "
  + ("The host page cache (block services' files, shard databases) was dropped before every "
     "timed run (`drop_caches` 3, which also drops dentries and inodes), so each starts cold. "
     if cold == "1" else
     "No caches were dropped (no passwordless sudo): an untimed read of both files first "
     "warmed the host page cache that holds the block services' files. ")
  + 
  "Rustern is mounted next to ternfuse (same path depth). "
  "Targets are recorded, not gated (spec: Performance Targets).",
  "",
  "## Throughput and CPU",
  "",
  "Throughput: higher is better. CPU (user + system time of the client process per GiB "
  "read): lower is better.",
  "",
  "| Workload | ternfuse MiB/s | Rustern MiB/s | Rustern / ternfuse | ternfuse CPU s/GiB | "
  "Rustern CPU s/GiB | Target |",
  "|---|---|---|---|---|---|---|",
]
targets = {"seq-read": ">= 0.5x; 1 -> 4 readers >= 2x; CPU <= 2x at 4 readers",
           "rand-read": ">= 0.5x"}
for label in labels:
    if not label.startswith(("seq-read", "rand-read")):
        continue
    a, b = get(label, "ternfuse"), get(label, "rustern")
    f = lambda r, k, fmt: format(r[k], fmt) if r else "–"
    t = targets["seq-read" if label.startswith("seq") else "rand-read"]
    lines.append(f"| {label} | {f(a,'mib_s','.0f')} | {f(b,'mib_s','.0f')} | "
                 f"{ratio(a,b,'mib_s')}{spread(a,b,'mib_s')} | "
                 f"{f(a,'cpu_s_per_gib','.2f')} | {f(b,'cpu_s_per_gib','.2f')} | {t} |")
s1, s4 = get("seq-read-1", "rustern"), get("seq-read-4", "rustern")
if s1 and s4:
    lines += ["", f"Rustern scaling 1 -> 4 readers: {s4['mib_s'] / s1['mib_s']:.2f}x (target >= 2x)."]
lines += [
  "",
  "## Memory",
  "",
  "Resident memory of the client process, MiB: lower is better. Start is `VmRSS` before the "
  "workload's first round; peak is the highest `VmHWM` of its rounds, each reset to that "
  "round's start first (`/proc/<pid>/clear_refs`), so a peak belongs to its own workload. Rustern runs with "
  "its default `--memory-limit` except in `-limit256M` rows. The ternfuse at <ref-mount> is "
  "long-lived, so its start includes memory kept from earlier use; the `-limit256M` ternfuse "
  "is mounted fresh.",
  "",
  "| Workload | ternfuse start | ternfuse peak | Rustern start | Rustern peak | "
  "peak Rustern / ternfuse |",
  "|---|---|---|---|---|---|",
]
for label in labels:
    a, b = get(label, "ternfuse"), get(label, "rustern")
    if not (a or b) or "hwm_mib" not in (a or b):
        continue
    f = lambda r, k: f"{r[k]:.0f}" if r else "–"
    lines.append(f"| {label} | {f(a,'rss0_mib')} | {f(a,'hwm_mib')} | {f(b,'rss0_mib')} | "
                 f"{f(b,'hwm_mib')} | {ratio(a,b,'hwm_mib')} |")
lines += [
  "",
  "## Latency",
  "",
  "fio rows: completion latency of one request. Metadata rows, per round: "
  f"{min(int(lat_n), 10000)} cold lookups (one per name), {lat_n} stats, {max(int(lat_n) // 50, 5)} "
  "cached listings and 20 changed listings of the 10,000-entry directory.",
  "",
  "| Metric | ternfuse p50 µs | ternfuse p99 µs | Rustern p50 µs | Rustern p99 µs | "
  "p50 Rustern / ternfuse | p99 Rustern / ternfuse | Target |",
  "|---|---|---|---|---|---|---|---|",
]
for label in labels:
    a, b = get(label, "ternfuse"), get(label, "rustern")
    if not (a or b) or "p50_us" not in (a or b):
        continue
    f = lambda r, k: (f"{r[k]:.2f}" if r[k] < 10 else f"{r[k]:.1f}") if r else "–"
    t = "p99 <= 2x" if not label.startswith(("seq", "rand")) else "recorded"
    lines.append(f"| {label} | {f(a,'p50_us')} | {f(a,'p99_us')} | {f(b,'p50_us')} | "
                 f"{f(b,'p99_us')} | {ratio(a,b,'p50_us')}{spread(a,b,'p50_us')} | "
                 f"{ratio(a,b,'p99_us')}{spread(a,b,'p99_us')} | {t} |")
lines += [
  "",
  f"UDP receive buffer drops during the run (`/proc/net/snmp` RcvbufErrors): {drops} "
  "(input for the socket2 idea in `docs/future-features.md`).",
  "",
  MARK,
]
# Hand-written notes below the marker (dhat, soak results) survive a rerun.
try:
    old = open(out).read()
    keep = old.split(MARK, 1)[1] if MARK in old else ""
except FileNotFoundError:
    keep = "\n"
open(out, "w").write("\n".join(lines) + keep)
print(f"bench: wrote {out}")
EOF
