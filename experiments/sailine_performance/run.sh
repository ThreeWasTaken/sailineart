#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ -z "${CARGO_HOME:-}" && -d "../.tooling/cargo" ]]; then export CARGO_HOME="$(realpath ../.tooling/cargo)"; fi
profile="${1:-quick}"
output="${2:-target/sailine-bench-${profile}.json}"
cache_bounds="${3:-content}"
case_index="${4:-}"
if [[ "$cache_bounds" != content && "$cache_bounds" != tile ]]; then echo "cache bounds must be content or tile" >&2; exit 2; fi
revision="$(git rev-parse HEAD)"
dirty=false
if ! git diff --quiet || [[ -n "$(git ls-files --others --exclude-standard)" ]]; then dirty=true; fi
cpu="$(awk -F: '/model name/ {sub(/^[ \t]+/, "", $2); print $2; exit}' /proc/cpuinfo 2>/dev/null || true)"
bench_args=(--profile "$profile" --output "$output")
if [[ -n "$case_index" ]]; then bench_args+=(--case "$case_index"); fi
SAILINE_GIT_COMMIT="$revision" SAILINE_WORKTREE_DIRTY="$dirty" SAILINE_CPU_MODEL="${cpu:-unknown}" SAILINE_CACHE_BOUNDS="$cache_bounds" \
  cargo run --release --offline -p photocraft-engine --features sailine-bench --example sailine_bench -- \
  "${bench_args[@]}"
printf 'Saved benchmark report to %s\n' "$output"
