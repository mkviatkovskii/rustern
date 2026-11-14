#!/usr/bin/env bash
# Start, stop or check the local TernFS test cluster (upstream ternrun).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
DATA_DIR=${RUSTERN_TEST_DATA_DIR:-$ROOT/ternfs/mnt}
DATA_DIR=$(realpath -m "$DATA_DIR")
PARENT=$(dirname "$DATA_DIR")
LOG=$PARENT/ternrun.log
PIDFILE=$PARENT/ternrun.pid
BIN=$ROOT/ternfs/build/alpine
REGISTRY=${RUSTERN_TEST_REGISTRY:-127.0.0.1:10001}

registry_up() {
  local host=${REGISTRY%:*} port=${REGISTRY##*:}
  timeout 2 bash -c "exec 3<>/dev/tcp/$host/$port" 2>/dev/null
}

cmd_start() {
  if registry_up; then
    echo "cluster already running ($REGISTRY)"
    return 0
  fi
  mkdir -p "$DATA_DIR"
  local fstype
  fstype=$(stat -f -c %T "$DATA_DIR")
  if [[ $fstype == tmpfs ]]; then
    echo "refusing: $DATA_DIR is on tmpfs" >&2
    exit 1
  fi
  local free_kb
  free_kb=$(df -Pk "$DATA_DIR" | awk 'NR==2 {print $4}')
  local min_gb=${RUSTERN_MIN_FREE_GB:-20}
  if ((free_kb < min_gb * 1000 * 1000)); then
    echo "refusing: less than $min_gb GB free under $DATA_DIR" >&2
    exit 1
  fi
  [[ -x $BIN/ternrun ]] || {
    echo "missing $BIN/ternrun (build upstream first)" >&2
    exit 1
  }
  unmount_stale
  : >"$LOG"
  # setsid: the cluster must outlive the shell (and session) that started it.
  setsid nohup "$BIN/ternrun" -leader-only -data-dir "$DATA_DIR" -binaries-dir "$BIN/" \
    >>"$LOG" 2>&1 &
  echo $! >"$PIDFILE"
  for _ in $(seq 120); do
    if grep -q operational "$LOG"; then
      echo "cluster operational (log: $LOG)"
      return 0
    fi
    if ! kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "ternrun exited; see $LOG" >&2
      exit 1
    fi
    sleep 1
  done
  echo "cluster not operational after 120 s; see $LOG" >&2
  exit 1
}

cmd_stop() {
  if [[ -f $PIDFILE ]]; then
    local pid
    pid=$(cat "$PIDFILE")
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid"
      for _ in $(seq 30); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
      done
    fi
    rm -f "$PIDFILE"
  fi
  unmount_stale
}

unmount_stale() {
  grep " fuse" /proc/self/mounts | awk '{print $2}' | while read -r m; do
    case $m in "$DATA_DIR"/*) fusermount3 -u "$m" || fusermount3 -uz "$m" || true ;; esac
  done
}

cmd_status() { registry_up; }

case ${1:-} in
  start) cmd_start ;;
  stop) cmd_stop ;;
  status) cmd_status ;;
  *)
    echo "usage: $0 start|stop|status" >&2
    exit 2
    ;;
esac
