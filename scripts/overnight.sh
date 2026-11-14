#!/usr/bin/env bash
# shellcheck disable=SC2317  # cleanup, killtree and unmount run from traps
# Long stability run (T125): correctness, memory and performance of a Rustern mount over
# hours, on a loopback network shaped with `tc netem`, without root.
#
# usage: scripts/overnight.sh [hours=8]
#
# The cluster and both clients run inside an unprivileged user + network + mount namespace
# (`unshare -Urnm`): there `tc` may shape the namespace's own loopback, so every packet
# (client <-> cluster and inside the cluster) gets the delay and loss, as on a real
# network. A running host cluster is stopped first (both use the same data directory) and
# started again at the end.
#
# The run cycles through phases of PHASE_MIN minutes each:
#   clean  no shaping
#   delay  20 ms +- 10 ms per packet (so about 40 ms round trip)
#   lossy  delay plus 1% loss
#   harsh  100 ms +- 20 ms, 5% loss in bursts, 1% duplicates, 5% reordered
#   stall  no shaping; every 60 s one ternblocks and one ternshard are SIGSTOPped for 10 s
# Each phase starts with a performance block (fio: sequential 1 MiB reads with 1 and 16
# readers, random 4 KiB and 128 KiB reads with 16), on Rustern and, with COMPARE=1, on a
# ternfuse mount under the same shaping. The rest of the phase is load: back-to-back
# `sha256sum -c corpus.sha256` passes plus a listing of the corpus compared with the one
# taken at the start, while fio reads randomly in the background. A pass still running
# PASS_GRACE seconds after its phase ends is killed and counted as incomplete. RSS, CPU, threads and
# open files of both daemons are sampled every 10 s; the metrics are dumped (SIGUSR1) at
# the end of every phase.
#
# FAIL when: a checksum or the listing is ever wrong; a read error happens in a clean,
# delay or lossy phase, or in a stall phase outside a fault window (window + the overall
# block deadline); a clean-phase pass is incomplete; ENOTCONN is seen; Rustern exits; peak
# RSS exceeds MAX_RSS_MIB. Read errors in harsh phases and incomplete passes in shaped
# phases are recorded, not failed. RSS growth between the first and the
# last clean phase is reported.
#
# Stop a run with SIGTERM or Ctrl-C: the namespace is torn down and the host cluster
# restarted.
#
# Results stay in target/overnight/<timestamp>/ (`latest` points at the newest run):
# `status` is rewritten every 10 s for watching a run; report.md is appended to
# benchmarks.md at the end.
#
# Environment: PHASE_MIN (30), PHASES ("clean delay lossy harsh stall"), PERF_SECONDS (45,
# per fio job), PASS_GRACE (300), COMPARE (1), MAX_RSS_MIB (2048), OUT (benchmarks.md path).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD

hours=${1:-8}
phase_min=${PHASE_MIN:-30}
read -r -a phases <<< "${PHASES:-clean delay lossy harsh stall}"
perf_s=${PERF_SECONDS:-45}
pass_grace=${PASS_GRACE:-300}
compare=${COMPARE:-1}
max_rss_mib=${MAX_RSS_MIB:-2048}
out=${OUT:-specs/001-fuse-client-poc/benchmarks.md}
block_deadline=60
registry=127.0.0.1:10001
BIN=$ROOT/ternfs/build/alpine
big=corpus/sizes/size_1073741824

need() { command -v "$1" >/dev/null || { echo "overnight: missing required tool: $1" >&2; exit 1; }; }

# ---- outside the namespace: build, stop the host cluster, run, restart, report ----
if [[ ${RUSTERN_OVERNIGHT_INSIDE:-} != 1 ]]; then
  for t in unshare tc ip fio fusermount3 python3 sha256sum cargo; do need "$t"; done
  [[ -x $BIN/ternrun && -x $BIN/ternfuse ]] || { echo "overnight: build upstream first" >&2; exit 1; }
  cargo build --release -q -p rustern
  dir=$ROOT/target/overnight/$(date +%Y%m%d-%H%M%S)
  mkdir -p "$dir"
  ln -sfn "$dir" "$ROOT/target/overnight/latest"
  host_up=0
  if scripts/cluster.sh status >/dev/null 2>&1; then
    host_up=1
    echo "overnight: stopping the host cluster (restarted at the end)"
    scripts/cluster.sh stop
  fi
  # Without --fork, unshare execs the inner half in its own process: a signal sent to that
  # PID reaches the inner trap, which tears the namespace down before we go on.
  RUSTERN_OVERNIGHT_INSIDE=1 RUSTERN_OVERNIGHT_DIR=$dir \
    unshare -Urnm --propagation private "$0" "$hours" &
  upid=$!
  trap 'kill -TERM "$upid" 2>/dev/null || true' INT TERM
  rc=0
  while :; do
    if wait "$upid"; then rc=0; else rc=$?; fi
    kill -0 "$upid" 2>/dev/null || break   # still running: wait was cut short by a signal
  done
  trap - INT TERM
  echo "overnight: inner half exited with $rc"
  if (( host_up )); then scripts/cluster.sh start || echo "overnight: host cluster did not restart" >&2; fi
  if [[ -f $dir/report.md ]]; then cat "$dir/report.md" >> "$out"; cat "$dir/report.md"; fi
  echo "overnight: results in $dir"
  exit "$rc"
fi

# ---- inside the namespace ----
dir=$RUSTERN_OVERNIGHT_DIR
mnt=$dir/rustern-mnt
tmnt=$dir/ternfuse-mnt
mkdir -p "$mnt" "$tmnt"
pid='' tpid=''
log() { echo "$(date +%T) $*" | tee -a "$dir/events"; }
unmount() { # no `return` here: in a function called from the EXIT trap it ends the trap
  local n=0
  until fusermount3 -u "$1" 2>/dev/null; do
    if (( ++n == 50 )); then fusermount3 -uz "$1" || true; break; fi
    sleep 0.1
  done
}
killtree() { # <pid>: the process first, so a loop cannot start new children
  local kids c
  kids=$(pgrep -P "$1" || true)
  kill "$1" 2>/dev/null || true
  for c in $kids; do killtree "$c"; done
}
cleaned=0
cleanup() {
  # Once, and in the main shell only, whatever else ends up running the EXIT trap.
  [[ $BASHPID == "$$" ]] && (( !cleaned )) || return 0
  cleaned=1
  trap '' INT TERM
  local j js
  js=$(jobs -p)
  log "cleanup: stopping jobs"
  for j in $js; do [[ $j == "$pid" || $j == "$tpid" ]] || killtree "$j"; done
  if [[ -f $dir/stopped ]]; then xargs -r kill -CONT < "$dir/stopped" 2>/dev/null || true; fi
  tc qdisc del dev lo root 2>/dev/null || true
  [[ -n $pid ]] && { kill -TERM "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true; }
  mountpoint -q "$mnt" && unmount "$mnt"
  mountpoint -q "$tmnt" && unmount "$tmnt"
  [[ -n $tpid ]] && { kill -TERM "$tpid" 2>/dev/null || true; }
  log "cleanup: clients stopped; stopping the cluster"
  scripts/cluster.sh stop >> "$dir/cluster.log" 2>&1 || true
  log "cleanup: done"
}
trap cleanup EXIT
trap 'log "stopped by a signal"; exit 130' INT TERM

ip link set lo up
log "starting the cluster in the namespace"
scripts/cluster.sh start >> "$dir/cluster.log" 2>&1

target/release/rustern -registry "$registry" -log-file "$dir/rustern.log" "$mnt" &
pid=$!
for _ in $(seq 100); do mountpoint -q "$mnt" && break; sleep 0.1; done
mountpoint -q "$mnt" || { log "rustern did not mount"; exit 1; }
if (( compare )); then
  "$BIN/ternfuse" -log-file "$dir/ternfuse.log" -registry "$registry" "$tmnt" &
  tpid=$!
  for _ in $(seq 100); do mountpoint -q "$tmnt" && break; sleep 0.1; done
  mountpoint -q "$tmnt" || { log "ternfuse did not mount"; exit 1; }
fi
[[ -f $mnt/corpus/corpus.sha256 ]] || { log "no corpus; run scripts/make-corpus.sh"; exit 1; }
listing() { # <mount> [time limit, s]
  (cd "$1/corpus" && timeout -k 10 "${2:-600}" find . -printf '%y %p %s\n' | LC_ALL=C sort)
}
listing "$mnt" > "$dir/listing.ref"
log "mounted; corpus has $(wc -l < "$dir/listing.ref") entries"

mapfile -t blocks < <(pgrep -x ternblocks || true)
mapfile -t shards < <(pgrep -x ternshard || true)

shape() { # <phase>
  tc qdisc del dev lo root 2>/dev/null || true
  local args
  case $1 in
    delay) args="delay 20ms 10ms distribution normal" ;;
    lossy) args="delay 20ms 10ms distribution normal loss 1%" ;;
    harsh) args="delay 100ms 20ms distribution normal loss 5% 25% duplicate 1% reorder 5% 50%" ;;
    *) return 0 ;;
  esac
  # shellcheck disable=SC2086
  tc qdisc add dev lo root netem limit 100000 $args
}

# fio_job <phase> <cycle> <client> <mount> <label> <fio args...>: one JSON line to perf.jsonl.
fio_job() {
  local phase=$1 cycle=$2 client=$3 m=$4 label=$5 t0; shift 5
  t0=$(date +%s)
  # In the background: bash runs a signal trap only once a foreground command ends.
  fio --output-format=json --readonly --direct=1 --invalidate=1 --ioengine=psync \
    --group_reporting --name="$label" --filename="$m/$big" --size=1G --time_based \
    --runtime="$perf_s" "$@" > "$dir/fio.json" 2> "$dir/fio.err" &
  if wait $!; then
    python3 - "$phase" "$cycle" "$client" "$label" "$dir/fio.json" $(( $(date +%s) - t0 )) \
      >> "$dir/perf.jsonl" <<'EOF'
import json, sys
phase, cycle, client, label, path, wall = sys.argv[1:]
r = json.load(open(path))["jobs"][0]["read"]
p = r["clat_ns"]["percentile"]
print(json.dumps({"phase": phase, "cycle": int(cycle), "client": client, "label": label,
  "mib_s": r["bw_bytes"] / 2**20, "iops": r["iops"], "p50_us": p["50.000000"] / 1e3,
  "p99_us": p["99.000000"] / 1e3, "max_us": r["clat_ns"]["max"] / 1e3, "wall_s": int(wall),
  "errors": 0}))
EOF
  else
    printf '{"phase": "%s", "cycle": %s, "client": "%s", "label": "%s", "errors": 1}\n' \
      "$phase" "$cycle" "$client" "$label" >> "$dir/perf.jsonl"
    log "fio $label on $client failed: $(tail -1 "$dir/fio.err")"
  fi
}

perf_block() { # <phase> <cycle>
  local clients=(rustern) c
  (( compare )) && { if (( $2 % 2 )); then clients=(ternfuse rustern); else clients=(rustern ternfuse); fi; }
  for c in "${clients[@]}"; do
    local m=$mnt; [[ $c == ternfuse ]] && m=$tmnt
    fio_job "$1" "$2" "$c" "$m" seq-read-1 --rw=read --bs=1M --numjobs=1
    fio_job "$1" "$2" "$c" "$m" seq-read-16 --rw=read --bs=1M --numjobs=16
    fio_job "$1" "$2" "$c" "$m" rand-read-4k --rw=randread --bs=4k --numjobs=16
    fio_job "$1" "$2" "$c" "$m" rand-read-128k --rw=randread --bs=128k --numjobs=16
  done
}

# Checksum passes until <end>, each cut off PASS_GRACE s after it: "<phase> <cycle> <start>
# <end> <wrong> <read errors> <listing ok> <incomplete>". The listing of a cut-off pass is
# not compared.
check_loop() { # <phase> <cycle> <end>
  while (( $(date +%s) < $3 )); do
    local s e wrong ioerr lst=1 cut=0 rc=0
    s=$(date +%s)
    (cd "$mnt/corpus" && exec timeout -k 10 $(( $3 + pass_grace - s )) \
      sha256sum -c --quiet corpus.sha256) > "$dir/pass.out" 2>&1 || rc=$?
    (( rc == 124 || rc == 137 )) && cut=1
    if (( !cut )); then
      rc=0
      listing "$mnt" $(( $3 + pass_grace - $(date +%s) > 10 ? $3 + pass_grace - $(date +%s) : 10 )) \
        > "$dir/listing.now" 2>> "$dir/pass.out" || rc=$?
      if (( rc == 124 || rc == 137 )); then cut=1
      else cmp -s "$dir/listing.ref" "$dir/listing.now" || lst=0; fi
    fi
    e=$(date +%s)
    wrong=$(grep -c ': FAILED$' "$dir/pass.out" || true)
    ioerr=$(grep -cE 'FAILED open or read|Input/output error|No such file' "$dir/pass.out" || true)
    if (( wrong || ioerr || !lst )); then
      { echo "== $1 cycle $2 at $s"; head -50 "$dir/pass.out"; } >> "$dir/errors"
      (( lst )) || diff "$dir/listing.ref" "$dir/listing.now" | head -20 >> "$dir/errors" || true
    fi
    grep -qs 'Transport endpoint is not connected' "$dir/pass.out" && touch "$dir/enotconn"
    (( cut )) && log "$1 cycle $2: checksum pass cut off after $(( e - s )) s"
    echo "$1 $2 $s $e $wrong $ioerr $lst $cut" >> "$dir/passes"
  done
}

fault_loop() { # <end>: SIGSTOP one block service and one shard for 10 s every 60 s
  while (( $(date +%s) + 60 < $1 )); do
    sleep 50
    local b=${blocks[RANDOM % ${#blocks[@]}]} s=${shards[RANDOM % ${#shards[@]}]} t0
    t0=$(date +%s)
    echo "$b $s" > "$dir/stopped"   # a subshell: cleanup reads the file, not a variable
    kill -STOP "$b" "$s"
    sleep 10
    kill -CONT "$b" "$s"; rm -f "$dir/stopped"
    echo "$t0 $(date +%s)" >> "$dir/faults"
  done
}

sample() { # <phase> <cycle>: one line per daemon to samples.tsv
  local c p
  for c in rustern ternfuse; do
    p=$pid; [[ $c == ternfuse ]] && p=$tpid
    [[ -n $p && -d /proc/$p ]] || continue
    echo "$(date +%s) $1 $2 $c $(awk '/^VmRSS/ {r=$2} /^VmHWM/ {h=$2} /^Threads/ {t=$2}
      END {print r, h, t}' "/proc/$p/status") $(awk '{print $14 + $15}' "/proc/$p/stat") \
      $(find "/proc/$p/fd" -mindepth 1 -maxdepth 1 | wc -l)" >> "$dir/samples.tsv"
  done
}

start=$(date +%s)
end_all=$(( start + ${hours%.*} * 3600 ))
[[ $hours == *.* ]] && end_all=$(( start + $(awk -v h="$hours" 'BEGIN {print int(h * 3600)}') ))
i=0 cycle=0
while (( $(date +%s) + 120 < end_all )); do
  ph=${phases[i % ${#phases[@]}]}
  (( i % ${#phases[@]} == 0 )) && cycle=$((cycle + 1))
  pend=$(( $(date +%s) + phase_min * 60 )); (( pend > end_all )) && pend=$end_all
  log "phase $ph (cycle $cycle) until $(date -d "@$pend" +%T)"
  shape "$ph"
  perf_block "$ph" "$cycle"
  load_s=$(( pend - $(date +%s) ))
  if (( load_s > 30 )); then
    fio --name=load --readonly --direct=1 --ioengine=psync --rw=randread --bs=128k \
      --filename="$mnt/$big" --size=1G --numjobs=4 --time_based --runtime="$load_s" \
      --output="$dir/load.out" 2>> "$dir/load.err" &
    lpid=$!
    check_loop "$ph" "$cycle" "$pend" &
    cpid=$!
    fpid=
    if [[ $ph == stall ]]; then fault_loop "$pend" & fpid=$!; fi
    while (( $(date +%s) < pend )); do
      sample "$ph" "$cycle"
      kill -0 "$pid" 2>/dev/null || break
      python3 scripts/overnight_report.py --status "$dir" "$start" "$end_all" > "$dir/status.tmp" \
        && mv "$dir/status.tmp" "$dir/status"
      sleep 10 & wait $!
    done
    wait "$cpid" "$lpid" ${fpid:+"$fpid"} 2>/dev/null || true
    grep -qs 'Transport endpoint is not connected' "$dir/load.err" && touch "$dir/enotconn"
  fi
  shape clean
  sample "$ph" "$cycle"
  echo "$ph $cycle $(awk '/^VmRSS/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)" \
    >> "$dir/phase_rss"
  kill -USR1 "$pid" 2>/dev/null || true
  if ! kill -0 "$pid" 2>/dev/null; then log "rustern exited"; touch "$dir/exited"; break; fi
  i=$((i + 1))
done
log "done"
stat "$mnt" >/dev/null 2>&1 || touch "$dir/dead"
rc=0
python3 scripts/overnight_report.py --final "$dir" "$start" "$(date +%s)" \
  --deadline "$block_deadline" --max-rss "$max_rss_mib" > "$dir/report.md" || rc=$?
exit "$rc"
