#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -z "${CARGO_HOME:-}" && -d "$repo/../.tooling/cargo" ]]; then export CARGO_HOME="$repo/../.tooling/cargo"; fi
output="${1:-$repo/experiments/sailine_performance/overlay.json}"
if [[ "$output" != /* ]]; then output="$repo/$output"; fi
CARGO_PROFILE_TEST_PACKAGE_PHOTOCRAFT_UI_EGUI_OPT_LEVEL="${CARGO_PROFILE_TEST_PACKAGE_PHOTOCRAFT_UI_EGUI_OPT_LEVEL:-3}" SAILINE_BENCH_OUTPUT="$output" cargo test --offline -p photocraft-ui-egui sailine_overlay_bench -- --ignored --nocapture
printf 'Saved overlay benchmark report to %s\n' "$output"
