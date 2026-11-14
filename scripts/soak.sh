#!/usr/bin/env bash
# Soak test (SC-003): a mixed read load on a Rustern mount while block services and shards
# are stopped and resumed.
#
# usage: scripts/soak.sh [minutes=30]
#
# Mounts Rustern on a temporary directory, runs fio random reads plus repeated
# `sha256sum -c corpus.sha256`, and every 60 s SIGSTOPs one random ternblocks and one
# random ternshard process for 10 s. Passes when, at the end:
# - the mount is alive and no ENOTCONN was ever seen;
# - no checksum ever mismatched (a wrong byte), and every read error happened in a pass
#   that overlapped a fault window (window + the overall block deadline);
# - peak RSS stayed below --memory-limit + 256 MiB (catches leaks).
# Appends the result to specs/001-fuse-client-poc/benchmarks.md.
#
# Environment: RUSTERN_REGISTRY (127.0.0.1:10001), RUSTERN_TEST_DATA_DIR (ternfs/mnt),
# MEMORY_LIMIT_MIB (1024).
set -euo pipefail
cd "$(dirname "$0")/.."

minutes=${1:-30}
registry=${RUSTERN_REGISTRY:-127.0.0.1:10001}
data=${RUSTERN_TEST_DATA_DIR:-ternfs/mnt}
limit_mib=${MEMORY_LIMIT_MIB:-1024}
block_deadline=60
out=specs/001-fuse-client-poc/benchmarks.md

need() { command -v "$1" >/dev/null || {
  echo "soak: missing required tool: $1" >&2
  exit 1
}; }
need fio
need fusermount3
need cargo
need sha256sum
[[ -f $data/fuse_loc1/mnt/corpus/corpus.sha256 ]] ||
  {
    echo "soak: no corpus; run scripts/make-corpus.sh" >&2
    exit 1
  }
mapfile -t blocks < <(pgrep -x ternblocks || true)
mapfile -t shards < <(pgrep -x ternshard || true)
((${#blocks[@]} && ${#shards[@]})) || {
  echo "soak: cluster not running" >&2
  exit 1
}

cargo build --release -q -p rustern
work=$(mktemp -d "${TMPDIR:-/tmp}/rustern-soak.XXXXXX")
mnt=$work/mnt
mkdir "$mnt"
stopped=()
pid=
cleanup() {
  for p in "${stopped[@]}"; do kill -CONT "$p" 2>/dev/null || true; done
  jobs -p | xargs -r kill 2>/dev/null || true
  [[ -n $pid ]] && {
    kill -TERM "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
  }
  mountpoint -q "$mnt" && fusermount3 -u "$mnt"
  rm -rf -- "$work"
}
trap cleanup EXIT

target/release/rustern -registry "$registry" -log-file "$work/rustern.log" \
  --memory-limit "${limit_mib}MiB" "$mnt" &
pid=$!
for _ in $(seq 100); do
  mountpoint -q "$mnt" && break
  sleep 0.1
done
mountpoint -q "$mnt" || {
  echo "soak: rustern did not mount" >&2
  exit 1
}

end=$(($(date +%s) + minutes * 60))
echo "soak: $minutes min on $mnt (rustern pid $pid)"

fio --name=soak --readonly --direct=1 --ioengine=psync --rw=randread --bs=128k \
  --filename="$mnt/corpus/sizes/size_1073741824" --size=1G --numjobs=4 \
  --time_based --runtime=$((minutes * 60)) --output="$work/fio.out" 2>"$work/fio.err" &

# Each pass: "<start> <end> <wrong-hash count> <read-error count>".
(
  while (($(date +%s) < end)); do
    s=$(date +%s)
    (cd "$mnt/corpus" && sha256sum -c --quiet corpus.sha256) >"$work/pass.out" 2>&1 || true
    e=$(date +%s)
    cat "$work/pass.out" >>"$work/sha.out"
    wrong=$(grep -c ': FAILED$' "$work/pass.out" || true)
    ioerr=$(grep -c 'FAILED open or read' "$work/pass.out" || true)
    echo "$s $e $wrong $ioerr" >>"$work/passes"
  done
) &

# Fault windows: "<start> <end>".
while (($(date +%s) + 60 < end)); do
  sleep 50
  b=${blocks[RANDOM % ${#blocks[@]}]}
  s=${shards[RANDOM % ${#shards[@]}]}
  t0=$(date +%s)
  kill -STOP "$b" "$s"
  stopped=("$b" "$s")
  sleep 10
  kill -CONT "$b" "$s"
  stopped=()
  echo "$t0 $(date +%s)" >>"$work/faults"
  echo "soak: stopped ternblocks $b and ternshard $s for 10 s"
done
mapfile -t load < <(jobs -p | grep -vx "$pid")
wait "${load[@]}" 2>/dev/null || true

fail=()
stat "$mnt" >/dev/null 2>&1 || fail+=("mount is not alive")
kill -0 "$pid" 2>/dev/null || fail+=("rustern exited")
if grep -qs 'Transport endpoint is not connected' "$work/sha.out" "$work/fio.err" "$work/fio.out"; then
  fail+=("ENOTCONN seen")
fi
passes=0 wrong_total=0 ioerr_total=0
touch "$work/faults" "$work/passes"
while read -r s e wrong ioerr; do
  passes=$((passes + 1)) wrong_total=$((wrong_total + wrong)) ioerr_total=$((ioerr_total + ioerr))
  ((wrong == 0)) || fail+=("pass at $s: $wrong wrong checksums")
  if ((ioerr > 0)); then
    overlap=$(awk -v s="$s" -v e="$e" -v d="$block_deadline" \
      '$1 <= e && $2 + d >= s {n++} END {print n + 0}' "$work/faults")
    ((overlap > 0)) || fail+=("pass at $s: $ioerr read errors outside any fault window")
  fi
done <"$work/passes"
hwm=$(awk '/^VmHWM/ {print int($2 / 1024)}' "/proc/$pid/status" 2>/dev/null || echo 0)
((hwm < limit_mib + 256)) || fail+=("peak RSS $hwm MiB >= limit $limit_mib MiB + 256 MiB")
alerts=$(grep -c 'alert=true' "$work/rustern.log" || true)

result=PASS
((${#fail[@]})) && result=FAIL
{
  echo
  echo "## Soak ($(date +%F), $minutes min): $result"
  echo
  echo "- $(wc -l <"$work/faults") fault windows, $passes checksum passes, $wrong_total wrong"
  echo "  checksums, $ioerr_total read errors (all inside fault windows when PASS)."
  echo "- Peak RSS $hwm MiB with \`--memory-limit ${limit_mib}MiB\`; $alerts alert log lines."
  for f in "${fail[@]}"; do echo "- FAIL: $f"; done
} | tee -a "$out"
[[ $result == PASS ]]
