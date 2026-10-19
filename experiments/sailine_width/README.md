# SaiPressure reverse-engineering bench

This isolated experiment measures the six Sai 1.2.6 Beta 3 pressure examples in
`reference.png`. It is reproducible from the repository root:

```sh
CARGO_HOME=/mnt/SSD2/Codex/.tooling/cargo cargo run --manifest-path experiments/sailine_width/Cargo.toml --offline -- --prepare
python3 experiments/sailine_width/measure_normals.py
CARGO_HOME=/mnt/SSD2/Codex/.tooling/cargo cargo run --manifest-path experiments/sailine_width/Cargo.toml --offline
python3 experiments/sailine_width/render_diagnostics.py
```

`extract.py` records the grid and control anchors in `dataset.json`. Interior anchors are detected
from their green overlay markers; yellow endpoint centers were inspected against the screenshot
and recorded manually. Red pixels are segmented from the source capture. `--prepare` reconstructs
centerlines with the exact production `SaiSimplified` vector routine and records local normals.
`measure_normals.py` samples the red mask at half-pixel intervals across those normals, excluding
curve ends where the tangent is ill-defined. The data retains the source pixel coordinates,
normalized coordinates, raw red components, width profiles, and an estimated ±1.5 px uncertainty.

`diagnostic.png` overlays control markers, the reconstructed centerlines and the normal sampling
locations on the reference. `comparison_board.png` overlays the measured Sai silhouette in red and
the reconstructed linear-arc-length pressure outline in blue; overlap appears purple. The Rust
bench compares linear pressure in Bézier parameter, linear pressure in centerline arc length,
smoothstep, local cubic Hermite, and monotone cubic Hermite. Scores are case-balanced so the long
multi-point curves do not dominate.

## Current result

Linear interpolation in normalized segment arc length ranks first at **1.302 px mean absolute
width error** (16.12% after normalization by base width), across all six cases. Linear interpolation
in Bézier parameter is close at **1.373 px** (16.73%). The 0.071 px difference is much smaller than
the estimated extraction/antialiasing uncertainty, so these references do not establish that Sai
uses arc length. Smoothstep is visibly and numerically worse at 2.022 px; cubic candidates do not
improve the aggregate and the unconstrained local cubic overshoots. The current implementation
therefore keeps the simpler linear-in-parameter pressure profile provisionally; the arc-length
candidate remains the best measured fit and should be part of manual comparison in PhotoCraft.

All six examples have usable normal samples (97, 96, 470, 275, 291, 270). The automated proximity
filter marked zero samples ambiguous; this does not prove every pixel in the close loops is
unoccluded, and overlapping outlines, antialiasing, and the approximate centerline remain error
sources. Case 1's very wide, zero-to-200% taper is particularly useful for distinguishing profile
laws, while cases 3–6 test pressure changes along curved paths. No proprietary Sai formula is
claimed.

## PhotoCraft integration state

The production centerline default is now the validated `SaiSimplified` formula. Point pressure is
stored as the existing dimensionless width factor (100% = 1.0); absent serialized values still
default to 100%. The separate tool is named **SaiPressure**. Both SaiCurve and SaiPressure share
Ctrl-click insertion and Ctrl-drag point editing. SaiPressure changes the selected point only while
the user drags horizontally; a click alone selects and does not edit. Dragging right increases and
dragging left decreases pressure, clamped to 0–200%, previewed during the gesture (canvas rerender capped at about 30 Hz to keep dragging responsive) and committed as
one undo step on release. The base path-width control remains independent.

The width renderer makes an offset outline from the interpolated centerline, interpolates pressure
linearly by normalized arc length within each segment, and adds a round disk at each nonzero open
endpoint with the corresponding effective radius. Arc-length pressure was chosen after interactive
testing showed a slight endpoint swelling with Bézier-parameter interpolation; the six-reference
benchmark also ranked arc length first, though its 0.071 px lead is within extraction uncertainty.
This remains a working visual implementation, not a confirmed pixel-equivalent Sai
join/overlap rasterizer. Engine tests cover 0–200% pressure, insertion, undo, redo, serialization,
compatibility defaults, arc-length progress, and effective cap diameter. UI tests cover the shared
shortcuts and click vs drag behavior. The six-capture experiment is a raster comparison, not a
statistical identification: one screenshot scale is inferred from the grid and the center path is
reconstructed from the validated geometric model.

## Key files

- `dataset.json`: six anchors, pressure values, grid fit, reference contours and normal widths.
- `diagnostic.png`: extraction and normal-sampling review image.
- `comparison_board.png`: Sai silhouette vs predicted pressure outline.
- `results.json`: per-case candidate scores and aggregate ranking.
- `src/main.rs`: standalone Rust model scorer, linked directly to `photocraft-vector`.
- `extract.py`, `measure_normals.py`, `render_diagnostics.py`: extraction and report scripts.
