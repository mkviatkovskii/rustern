#!/usr/bin/env bash
# Builds the reference corpus in a local temp directory and copies it into <dest> (normally
# inside the ternfuse mount) with one `cp`. Nothing is ever redirected into <dest>: shell
# redirects into a ternfuse mount fail (see quickstart.md).
#
# usage: scripts/make-corpus.sh [--force] <dest>
set -euo pipefail

force=0
[[ ${1:-} == --force ]] && { force=1; shift; }
dest=${1:?usage: make-corpus.sh [--force] <dest>}
if [[ -e $dest ]]; then
  (( force )) || { echo "make-corpus: $dest exists (use --force to replace it)" >&2; exit 1; }
  rm -rf -- "$dest"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/rustern-corpus.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
src=$tmp/corpus
mkdir -p "$src"

rnd() { head -c "$1" /dev/urandom > "$2"; }

# Sizes around the inline (256 B), mirrored-span (65,536 B) and RS(10,4) (5,120,000 B)
# limits, plus multi-span and 1 GiB files.
mkdir "$src/sizes"
for n in 0 1 255 256 4095 4096 4097 65536 65537 1048576 5120000 5120001 30000000 \
         104857600 1073741824; do
  rnd "$n" "$src/sizes/size_$n"
done

# 100 directories: 20 chains tree/dNN/a/b/c/d (depth 5), 10 small files in each.
for t in $(seq -w 0 19); do
  d=$src/tree/d$t
  for sub in "" /a /a/b /a/b/c /a/b/c/d; do
    mkdir -p "$d$sub"
    for f in $(seq 0 9); do
      rnd $(( (10#$t * 397 + f * 811) % 9000 )) "$d$sub/f$f"
    done
  done
done

# One directory with 10,000 entries.
mkdir "$src/big"
for i in $(seq 0 9999); do
  rnd $(( i % 300 )) "$src/big/e$i"
done

# Names: 255 bytes, non-UTF-8.
mkdir "$src/names"
rnd 100 "$src/names/$(printf 'n%.0s' $(seq 1 255))"
rnd 100 "$src/names/"$'\xff\xfe'
rnd 100 "$src/names/bad"$'\xff\xfe'"name"

# Symlinks: relative, absolute, dangling, to a directory. And an empty directory.
mkdir "$src/links" "$src/empty"
ln -s ../sizes/size_1 "$src/links/relative"
ln -s /etc/passwd "$src/links/absolute"
ln -s does-not-exist "$src/links/dangling"
ln -s ../tree "$src/links/dir"

echo "make-corpus: copying into $dest"
cp -rP -- "$src" "$dest"

echo "make-corpus: hashing $dest"
(cd "$dest" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum) > "$tmp/corpus.sha256"
cp -- "$tmp/corpus.sha256" "$dest/corpus.sha256"
echo "make-corpus: $(wc -l < "$tmp/corpus.sha256") files in $dest"
