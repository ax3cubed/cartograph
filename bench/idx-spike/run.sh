#!/usr/bin/env bash
# IDX-02: every indexer-side phase 0 measurement, ten runs or more each, raw JSON lines out.
#
#   bench/idx-spike/run.sh <work-dir>
#
# <work-dir> holds the remotion clone (about 4 GB), the freshness fixture, the SQLite files and the
# results. Keep it outside the repository. Fixtures are public repos: read as data, never built or run.
set -euo pipefail

WORK=${1:?usage: run.sh <work-dir>}
HERE=$(cd "$(dirname "$0")" && pwd)
RUNS=${RUNS:-10}
REV=2b1b0c9feb7b7475bac4b7aaf9b36bfd7241e328 # remotion, as in docs/research/README.md
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$WORK/target}
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null

mkdir -p "$WORK/repos" "$WORK/results/idx02"
OUT="$WORK/results/idx02"
REMOTION="$WORK/repos/remotion.git"

# Pinned fixture: a bare clone that contains 2b1b0c9 (everything is read at that commit).
if [ ! -d "$REMOTION" ]; then
  git clone --bare --single-branch https://github.com/remotion-dev/remotion.git "$REMOTION"
fi
git --git-dir="$REMOTION" cat-file -e "$REV^{commit}"
# Walks and freshness start without a commit-graph, as a fresh clone has; D1 adds one later.
rm -rf "$REMOTION/objects/info/commit-graph" "$REMOTION/objects/info/commit-graphs"

cargo build --release --manifest-path "$HERE/../Cargo.toml" -p idx-spike
B="$CARGO_TARGET_DIR/release/idx-spike"

{
  echo "{\"machine\": \"$(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m)\", \"cores\": $(sysctl -n hw.ncpu 2>/dev/null || nproc), \"memory_bytes\": $(sysctl -n hw.memsize 2>/dev/null || echo 0), \"os\": \"$(uname -sr)\", \"git\": \"$(git --version)\", \"rustc\": \"$(rustc --version)\"}"
} > "$OUT/hardware.json"
load() { uptime | sed 's/.*load averages*: //'; }

# B7: walk and tree diff, interleaved so load drift hits every implementation alike.
: > "$OUT/walk.jsonl"
for i in $(seq 1 "$RUNS"); do
  for imp in git gix git2; do
    for r in "" "--renames"; do
      echo "{\"run\": $i, \"load\": \"$(load)\", \"result\": $($B walk "$REMOTION" "$REV" $imp $r)}" >> "$OUT/walk.jsonl"
    done
  done
done
# Agreement with git log, path by path and rename by rename, on every first-parent commit.
$B agree "$REMOTION" "$REV" > "$OUT/agree.json"

# H1: oxc_resolver over the tree at 2b1b0c9, then replay soundness on commit pairs.
$B resolve "$REMOTION" "$REV" --runs "$RUNS" > "$OUT/resolve.json"
: > "$OUT/replay.jsonl"
for gap in 1 50 2000; do
  $B replay "$REMOTION" "$REV" --pairs 24 --gap $gap >> "$OUT/replay.jsonl"
done

# D1: 30 repos (the decision), 43 and 50 beside it. remotion is one of them, indexed at 2b1b0c9.
FIX="$WORK/fresh"
for n in 30 43 50; do
  $B fresh-fixture "$FIX" $n --extra "$REMOTION@$REV" > /dev/null
done
# No network: denied by the sandbox, and the binary is checked for socket symbols.
NET=(sandbox-exec -p '(version 1)(allow default)(deny network*)')
nm -u "$B" | grep -E '_(connect|getaddrinfo|socket|sendto)$' > "$OUT/net-symbols.txt" || true
: > "$OUT/fresh.jsonl"
fresh_runs() { # <graph label> <repo counts...>
  local graph=$1; shift
  for n in "$@"; do
    for threads in 1 8; do
      # Ten separate processes: the first check in each is the cold one for that process.
      for i in $(seq 1 "$RUNS"); do
        echo "{\"graph\": \"$graph\", \"mode\": \"process\", \"load\": \"$(load)\", \"result\": $("${NET[@]}" $B fresh "$FIX/state-$n.json" --runs 1 --threads $threads)}" >> "$OUT/fresh.jsonl"
      done
      echo "{\"graph\": \"$graph\", \"mode\": \"in-process\", \"load\": \"$(load)\", \"result\": $("${NET[@]}" $B fresh "$FIX/state-$n.json" --runs $((RUNS * 3)) --threads $threads)}" >> "$OUT/fresh.jsonl"
    done
  done
}
fresh_runs none 30 43 50
# git writes a commit-graph on gc and maintenance; most long-lived clones have one. Measure with it.
git --git-dir="$REMOTION" commit-graph write --reachable
for r in "$FIX"/repo-*; do git -C "$r" commit-graph write --reachable; done
fresh_runs commit-graph 30 43 50

# Intervals in SQLite.
: > "$OUT/sqlite.jsonl"
for set in remotion synthetic; do
  $B sqlite "$WORK/sqlite" $set --runs "$RUNS" >> "$OUT/sqlite.jsonl"
done
echo "results in $OUT"
