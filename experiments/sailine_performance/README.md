# SaiLineArt performance experiments

Run from any directory with:

```sh
bash experiments/sailine_performance/run.sh quick
bash experiments/sailine_performance/run.sh full target/sailine-bench-full.json
bash experiments/sailine_performance/run-overlays.sh
```

This is a headless Release benchmark. It warms each operation, reports median, p95 and max over
at least 10 timed repetitions, and records the source revision, dirty-worktree flag, OS, CPU
architecture, CPU model and process peak RSS where `/proc` is available. `quick` covers a small
isolated case and a pressure/overlap case. `full` adds 100/1,000 paths, 100/1,000 points per path,
larger canvases and sparse/overlapping arrangements. The matrix deliberately avoids the full
Cartesian product because a million control points and repeated full raster rebuilds are not a
useful interactive workload.

The report separates centerline geometry, variable-width contour construction, one-path raster
rendering, document cloning, full `Session.execute` point/pressure/multi-point edits, and full
layer rebuilds. Command timings include history snapshots, cache maintenance and compositing;
the document-clone row provides a diagnostic baseline, not an additive decomposition.

The separate ignored egui test `sailine_overlay_bench`, run by `run-overlays.sh`, measures the
actual SaiPressure hover overlay (centerline skeleton and handles) for 10/100/1,000 points without
GPU presentation. Allocation counts are not recorded; peak RSS is a coarse process-level measure.

For a focused full case, use its zero-based case index (0..5) and choose the cache-bounds strategy
to compare former tile-based invalidation with optimized exact-content invalidation:

```sh
bash experiments/sailine_performance/run.sh full target/before.json tile 4
bash experiments/sailine_performance/run.sh full target/after.json content 4
```

The `tile` mode exists only under the benchmark feature and applies tile bounds to the current
compositor; it isolates bounds/recomposition cost, not the old per-path `Surface` memory layout.
Production always uses exact raster content bounds and compact per-path caches.

`baseline-quick.json` records the first pre-optimization quick run made on the SaiLineArt worktree
at PhotoCraft commit `ac2b2ba121abdce0171af9fb0de179e95207ff35`. The checkout had unrelated and
SaiLineArt edits pending, so this is a reproducible measurement of that source state only after
those changes are captured in Git; rerun on a clean commit before treating it as a release
baseline. p95 with 20 repetitions is meaningful as a sample summary but remains machine-specific.

See [report.md](report.md) for the before/after results, cache optimization rationale, subsystem
cost analysis, architecture map and the provisional PhotoCraft 0.10.0 migration assessment.
