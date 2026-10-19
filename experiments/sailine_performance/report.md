# SaiLineArt performance and migration audit

## Scope and reproducibility

The benchmarks are headless and run against the current pre-0.10.0 checkout. Commands are in
`README.md`; `run.sh quick` runs the small matrix and `run.sh full … content 2` selects the
1,000-path stress case. Reports include the Git revision, dirty-worktree state, host CPU, build
profile, warmups, repetitions, median, p95, maximum and process peak RSS. The current worktree
was already dirty before this audit, so these results describe the captured working source and
are not a clean release baseline. Repeat them after committing a stable source snapshot.

Host: AMD Ryzen 7 5800X, Linux x86_64, Rust Release profile. Engine timings use 3 warmups and
20 measurements in `quick`, 2 warmups and 10 measurements in `full`. UI overlay timings use 2
warmups and 20 measurements in an optimized test profile. GPU presentation is excluded.
Allocation counts are not available from this harness; RSS is a coarse process high-water mark.

## Results before optimization

The initial probe showed that the centerline calculation was not the drag bottleneck. For the
10-path, 100-point overlapped case, geometry was about 0.004 ms, while a full point move was
about 86 ms and a full rebuild about 98 ms. Rasterization and recomposition dominated.

The repeatable `final-before-tile-quick.json` run measured the tile-bounded compositor. In the
10×10 case, local recomposition was 15.443 ms median / 15.492 ms p95, point move 20.279 / 24.419
ms, and full rebuild 30.078 ms. In the 10×100 overlapped case, recomposition was 59.738 / 60.562
ms, point move 73.608 / 80.620 ms, and full rebuild 97.881 ms. Peak RSS was 131,460 KiB.

Exact content bounds reduced recomposition, but scanning the cached pixels to rediscover those
bounds on every edit remained unnecessary work. The 1,000-path, 10-point case also exposed a
larger issue: a separate tiled `Surface` per path peaked at 5,826,988 KiB. Long thin paths
reserved whole 256px tiles even when little of each tile contained stroke pixels.

## Measured optimization and results

Each per-path cache is now a compact row-major pixel buffer cropped to the raster's exact
non-transparent bounds. Its pixel data is held in `Arc<[f32]>`, so history snapshots cheaply
share it. Exact bounds travel with the cache rather than being rescanned for each composition.
The aggregate layer cache still uses PhotoCraft's tiled `Surface`; the renderer, interpolation,
pressure profile, caps, colors and stroke widths are unchanged.

| Scenario / operation | Tile baseline median / p95 | Compact content median / p95 |
|---|---:|---:|
| 10 paths × 10 points: local composition | 15.443 / 15.492 ms | 0.039 / 0.039 ms |
| 10 × 10: one-point edit command | 20.279 / 24.419 ms | 1.040 / 1.264 ms |
| 10 × 100 overlapped: local composition | 59.738 / 60.562 ms | 0.744 / 0.746 ms |
| 10 × 100 overlapped: one-point edit | 73.608 / 80.620 ms | 6.590 / 7.117 ms |
| 10 × 100 overlapped: multi-point edit | 113.615 ms median | 52.543 ms median |
| 10 × 100 overlapped: full rebuild | 97.881 ms median | 51.534 ms median |

For 1,000 paths × 10 points on a 2048 × 8192 canvas, the compact cache reduced peak RSS from
5,826,988 KiB to 891,564 KiB (about 85%). Median / p95 one-point edit was 3.870 / 4.207 ms
before and 4.106 / 4.415 ms after; this is within run-to-run variation and not a demonstrated
improvement. Full rebuild was 2,823.615 / 3,111.552 ms before and 2,683.176 / 2,728.230 ms
after. The selected-point stress operation improved from 349.794 / 388.870 ms to 266.899 /
268.222 ms. This operation updates up to 200 points across the many-path document and is a
deliberately heavy stress case, not a representative single drag.

The cache optimization is pixel-checked by `incremental_linework_cache_matches_full_rebuild_after_drag_and_pressure`, which compares the incremental cache with a full rebuild after move, pressure, multi-point edits, translation, undo and redo. The 16 Engine Linework tests pass. This supports exactness for those scenarios; it is not a proof for every possible clip or color format.

## Component costs and remaining bottlenecks

In the 1,000-path case, pure geometry was below 0.001 ms median for the tested 10-point path;
contour construction was 0.063 ms, one-path rasterization 1.627 ms, and local cache composition
0.714 ms. A document snapshot clone alone was 0.047 ms. The full edit command costs 3.8–4.6 ms
for a single point/pressure change. The discrepancy includes command dispatch, history and
cache invalidation, bounds/clip work, and raster generation; the current microbench does not
attribute each of those subcosts separately.

The optimized UI benchmark for the actual SaiPressure hover overlay measured 0.0099 ms median
for 10 points, 0.0646 ms for 100 and 0.6156 ms for 1,000. It measures CPU overlay construction,
not GPU submission or frame presentation. Overlay work scales with path point count and is a
reasonable follow-up target for generation-based caching, but current evidence does not justify
changing it yet. A follow-up measured overlays at zoom 0.25×, 1× and 4×: for 1,000 points the
medians were 0.605, 0.607 and 0.610 ms, respectively. This indicates point count dominates and
zoom has little effect in this path. Geometry and raster caches use document-space coordinates;
their cost is zoom-independent by design.

The compact buffer copies pixels once when a path is rasterized. The command still clones the
path list and map of cache entries while preparing an edit; pixel storage itself is shared.
Multi-point operations touching many paths remain expensive. Further edits to history ownership,
segment-level geometry invalidation, or overlay caching should wait for finer profiling and a
new pixel-equivalence test. The cache is rectangular; a long diagonal curve can have a large
bounding rectangle, so diagonal/multi-turn memory profiles should be added before claiming the
compact layout is optimal for every shape.

## SaiLineArt / PhotoCraft boundary

| Concern | Current home | Migration/maintenance implication |
|---|---|---|
| Interpolation, pressure contour, pointed joins | `crates/vector/src/linework.rs` | Closest part to reusable SaiLine Core; it still uses PhotoCraft geometry/color types. |
| Persistent path/point model and `LayerContent::Linework` | `crates/doc/src/linework.rs`, document `LayerContent` matches | Most invasive compatibility seam; new layer-tree or document invariants must preserve native editability. |
| Transient path/layer raster caches | `LineworkLayer` in `crates/doc/src/linework.rs`, commands in Engine | Current cache depends directly on `Surface`; model and render-adapter concerns are mixed. Moving all caches out now would be a broad change without benchmark proof. |
| Commands, undo/redo, composition and raster invalidation | `crates/engine/src/linework_cmds.rs` and Engine command registration | Port command semantics onto the new history/cache APIs and keep undo snapshots atomic. |
| Native `.pcraft` persistence | `crates/format/src/manifest.rs` and format conversion | Keep path controls/properties in the manifest; caches remain skipped/transient. Round-trip tests are essential. |
| PSD/export and rasterization | IO/PSD export paths and layer-content matches | Ensure editable native data is flattened only for exports that require raster output. Update exhaustive matches. |
| Tools, selection, Ctrl+T and overlays | `crates/ui-egui/src/sailine_ui.rs`, `canvas.rs`, `state.rs`, `tool_feedback.rs`, `transform_tool.rs`; tool registration/options | Highest UI conflict surface; reuse upstream pointer, Free Transform and options infrastructure where compatible. |

The practical separation is already partial: the vector crate owns geometry, Engine owns
commands and undo, and egui owns interaction. A useful future boundary would formalize
SaiLine Core around plain point/path properties and geometry; a PhotoCraft adapter would own
document layer, `Surface`, command/history, manifest and export integration; SaiLineArt UI
would own tools, selection, transforms and overlays. Do not move the persistent document type
out of `LayerContent` merely to match this diagram: it is what keeps `.pcraft` paths editable.

## PhotoCraft 0.10.0 compatibility review

The branch was not migrated. The local shallow tag fetch failed because GitHub DNS/network access
was unavailable, so an exact source-to-source diff and reliable Git auto-merge classification
could not be produced. The official release page identifies tag `v0.10.0` at signed commit
`8407d9c`, and its notes describe per-control pressure curves, Free Transform updates, Pen and
Curvature Pen editing, layer-tree listing, and related document/UI work. The pinned manifest is
available at the [official v0.10.0 Cargo.toml](https://raw.githubusercontent.com/storytold/photocraft/v0.10.0/Cargo.toml).
See [official v0.10.0 release notes](https://github.com/storytold/photocraft/releases/tag/v0.10.0).
That manifest also raises the project to Rust 1.95, edition 2024 and Cargo resolver 3, so the
migration host/toolchain and dependency lockfile need to be updated with the source port.

Given the local changes and those release features, these are risk-based expectations, not a
verified merge result:

| Classification | Areas |
|---|---|
| Likely automatic after exact diff | Isolated SaiLine interpolation and benchmark/docs files that do not overlap upstream edits. No file can be certified automatic without the tag diff. |
| Manual adaptation likely | Document layer model and exhaustive `LayerContent` matches; Engine commands and history snapshots; raster cache APIs; manifest conversion; PSD/export; UI event routing, tool registration and options. |
| Functional/visual regression risk | Editable `.pcraft` round-trip, undo grouping, SaiPressure profile rendering, partial line transforms, invalidation bounds, pointer capture and overlays. Free Transform and Pen/Curvature Pen changes may offer reusable primitives but can alter gesture semantics. |

The v0.10 per-control pressure work is a candidate reference for SaiPressure's UI/profile model;
compare data semantics and rendering before replacing SaiLineArt's validated interpolation.
The new Free Transform and Curvature Pen editing can inform shared gestures and handle visuals,
but SaiLineArt must continue transforming vector control positions only. The release's new layer
tree listing is a signal to inspect the document model and `LayerContent` changes directly during
the eventual local tag diff; the release note alone does not prove that the underlying model
changed in a particular way.

## Safe migration order

1. Obtain/verify the official tag locally and make a clean worktree from it; do not start by
   merging the dirty SaiLineArt worktree.
2. Diff all SaiLineArt-touched files against the exact tag and record upstream changes per file.
3. Port the document model and manifest conversion first; add save/load fixtures proving that
   points, width, pressure, color and pointed state survive and caches are rebuilt.
4. Port Engine commands, cache invalidation and undo/redo; run Linework and document tests.
5. Port UI/tool registration and event handling. Compare the new Free Transform and path-edit
   APIs before reusing them; retain SaiLineArt-specific semantics behind an adapter.
6. Port rasterization/export matches, then run PSD and rasterize-layer tests.
7. Build the full application, run the test matrix, compare benchmark and visual fixtures, and
   only then integrate the migration branch.

## Artifacts

`dataset` is synthetic and generated by the harness, not captured user artwork. JSON reports in
this directory preserve the benchmark samples, including baseline, exact-bounds and compact
cache runs, the 1,000-path stress case and UI overlays at multiple zooms. The scripts are the reproduction path;
do not compare reports across different machines as absolute performance guarantees.
