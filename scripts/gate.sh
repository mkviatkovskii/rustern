#!/usr/bin/env bash
# Merge gates. `--quick` skips everything that needs the local cluster.
set -euo pipefail
cd "$(dirname "$0")/.."

QUICK=0
[[ ${1:-} == --quick ]] && QUICK=1

need() { command -v "$1" >/dev/null || {
  echo "gate: missing required tool: $1" >&2
  exit 1
}; }
need cargo
need cargo-nextest
need cargo-deny
need go
need fusermount3
# CI runs shellcheck in its own step, which annotates the diff.
[[ -n ${GITHUB_ACTIONS:-} ]] || need shellcheck
need uvx
((QUICK)) || {
  need cargo-fuzz
  need cargo-llvm-cov
}

# Under GitHub Actions each step is a collapsible group in the log.
if [[ -n ${GITHUB_ACTIONS:-} ]]; then
  step() { printf '::endgroup::\n::group::%s\n' "$*"; }
else
  step() { printf '\n== %s\n' "$*"; }
fi

step "upstream pin"
head=$(git -C ternfs rev-parse HEAD)
pin=$(<.ternfs-rev)
[[ $head == "$pin" ]] || {
  echo "gate: ternfs/ is at $head, expected $pin (.ternfs-rev)" >&2
  exit 1
}

step "fmt"
cargo fmt --all --check
step "scripts"
[[ -n ${GITHUB_ACTIONS:-} ]] || shellcheck scripts/*.sh
go run mvdan.cc/sh/v3/cmd/shfmt@v3.14.1 -d scripts/*.sh
uvx ruff@0.17.5 check --quiet scripts .claude/hooks
uvx ruff@0.17.5 format --check --quiet scripts .claude/hooks
python3 -I .claude/hooks/test_prefer_serena.py
step "clippy"
cargo clippy --workspace --all-targets -- -D warnings
step "tests"
cargo nextest run --workspace --no-tests=warn
step "deny"
cargo deny --log-level error check

step "codec drift"
gen=crates/rustern-msgs/src/generated.rs
tmp=$(mktemp -d)
started=0 # set when this gate starts the cluster, so that it stops it again
on_exit() {
  rm -rf "$tmp"
  if ((started)); then scripts/cluster.sh stop; fi
}
trap on_exit EXIT
(cd tools && go run ./rsgen) >"$tmp/generated.rs"
diff -u "$gen" "$tmp/generated.rs" || {
  echo "gate: $gen is stale; rerun tools/rsgen" >&2
  exit 1
}
(cd tools && go run ./goldens "$tmp/goldens")
diff -r testdata/goldens "$tmp/goldens" >/dev/null ||
  {
    echo "gate: testdata/goldens is stale; rerun tools/goldens" >&2
    exit 1
  }

step "dependency direction"
users=$(cargo tree -e normal -i fuser --workspace --prefix none 2>/dev/null |
  awk '{print $1}' | grep '^rustern' | sort -u | tr '\n' ' ' || true)
case $users in
  "" | "rustern " | "rustern rustern-fuse " | "rustern-fuse ") ;;
  *)
    echo "gate: fuser reachable from: $users (only rustern-fuse and rustern allowed)" >&2
    exit 1
    ;;
esac

step "deviation tests exist"
# Every `path.rs::[mod::]name` or `path.rs::[mod::]{a, b}` in docs/deviations.md must name
# a test function in that file.
missing=0
# shellcheck disable=SC2016 # the grep pattern's backticks are literal
while IFS= read -r ref; do
  ref=${ref//\`/}
  file=${ref%%.rs::*}.rs
  rest=${ref#*.rs::}
  if [[ $rest == *"{"* ]]; then
    names=${rest#*\{}
    names=${names%\}}
  else names=${rest##*::}; fi
  for name in ${names//,/ }; do
    grep -qE "fn ${name}\(" "$file" 2>/dev/null ||
      {
        echo "gate: docs/deviations.md names $file::$name, which does not exist" >&2
        missing=1
      }
  done
done < <(grep -oE '`crates/[^`]+\.rs::[^`]+`' docs/deviations.md)
((missing == 0)) || exit 1

((QUICK)) && {
  step "quick gate passed"
  exit 0
}

step "fuzz smoke"
# Every target briefly, from its checked-in corpus; new inputs go to a scratch directory.
for t in crates/*/fuzz/fuzz_targets/*.rs; do
  crate=${t%%/fuzz/*}
  name=$(basename "$t" .rs)
  mkdir -p "$tmp/fuzz/$name"
  (cd "$crate" && cargo +nightly fuzz run "$name" "$tmp/fuzz/$name" "fuzz/corpus/$name" -- \
    -max_total_time="${RUSTERN_FUZZ_SECONDS:-20}" -print_final_stats=0)
done

step "cluster"
if ! scripts/cluster.sh status; then
  scripts/cluster.sh start
  started=1
fi
data=${RUSTERN_TEST_DATA_DIR:-ternfs/mnt}
[[ -d $data/fuse_loc1/mnt/corpus ]] || scripts/make-corpus.sh "$data/fuse_loc1/mnt/corpus"

# Every test runs again here, instrumented, so the coverage report covers the whole suite.
cargo llvm-cov clean --workspace
cov() { cargo llvm-cov nextest --no-report --workspace --run-ignored all --no-tests=warn "$@"; }
step "cluster tests"
# Fault tests corrupt blocks and stop cluster processes, so they must not overlap any other
# cluster test (a nextest test group only serialises its own members): two passes.
cov -E 'not test(/^cluster_serial::/)'
step "fault tests"
cov -E 'test(/^cluster_serial::/)'

step "coverage"
# The misses left are listed in docs/coverage.md: unreachable `?` on fixed-size encodes,
# checks the kernel makes first, OS failures and test-only panics. A drop below the floor
# means new code without tests.
cargo llvm-cov report --summary-only --fail-under-lines "${RUSTERN_COVERAGE_FLOOR:-98}"

step "gate passed"
[[ -z ${GITHUB_ACTIONS:-} ]] || echo "::endgroup::"
