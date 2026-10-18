#!/usr/bin/env python3
"""Extract observed red stroke measurements from the supplied SaiPressure screenshot.

Uses Python's standard library plus ImageMagick for PNG decoding/rendering. The screenshot
contains no separately identifiable control-point markers; cases 3–6 therefore carry
explicitly approximate anchor guesses and are excluded from parameter fitting.
"""
import hashlib
import json
import math
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "reference.png"
W, H = 1275, 859
ROIS = {
    1: [180, 190, 1030, 275], 2: [180, 365, 1010, 405],
    3: [40, 575, 310, 840], 4: [180, 405, 780, 600],
    5: [440, 600, 750, 750], 6: [900, 480, 1120, 745],
}
# Yellow endpoints and small green interior markers are visible in this capture. Coordinates
# are their manually reviewed pixel centers (the script's diagnostic overlays them).
ANCHORS = {
    1: ([(187, 232), (994, 232)], [0, 200], "yellow endpoint markers; ±1.5 px"),
    2: ([(187, 385), (994, 385)], [0, 200], "yellow endpoint markers; ±1.5 px"),
    3: ([(73, 807), (72, 614), (264, 614), (264, 806), (149, 805), (148, 691)], [100, 0, 100, 200, 100, 100], "yellow endpoints and four green anchors; route follows the red stroke; ±1.5 px"),
    4: ([(188, 500), (302, 422), (493, 578), (761, 462)], [0, 100, 100, 200], "yellow endpoints and two green anchors; ±1.5 px"),
    5: ([(455, 730), (571, 689), (724, 729), (724, 612)], [100, 200, 100, 100], "yellow endpoints and two green anchors; ±1.5 px"),
    6: ([(916, 730), (991, 498), (1031, 730), (1109, 501)], [100, 200, 0, 100], "yellow endpoints and two green anchors; ±1.5 px"),
}

def decode():
    raw = subprocess.check_output(["magick", str(SOURCE), "-depth", "8", "RGB:-"])
    if len(raw) != W * H * 3:
        raise ValueError("unexpected screenshot dimensions")
    return raw

def red_at(raw, x, y):
    i = 3 * (y * W + x)
    r, g, b = raw[i:i+3]
    return r > 190 and g < 95 and b < 95

def width_profile(raw, y_center, grid_origin, grid_period):
    # A horizontal centerline has vertical normals. Skip x positions on dark vertical gridlines,
    # then collect red support around the known centerline rather than measuring a screen-axis
    # slice on the curved examples.
    out = []
    for x in range(205, 985, 13):
        if abs((x - grid_origin) / grid_period - round((x - grid_origin) / grid_period)) < 0.09:
            continue
        ys = [y for y in range(y_center - 36, y_center + 37) if red_at(raw, x, y)]
        if not ys:
            continue
        # The component closest to the centreline; remove remote red pixels from nearby figures.
        groups = []
        for y in ys:
            if not groups or y > groups[-1][-1] + 1:
                groups.append([y])
            else:
                groups[-1].append(y)
        group = min(groups, key=lambda g: abs((g[0] + g[-1]) * 0.5 - y_center))
        if abs((group[0] + group[-1]) * 0.5 - y_center) > 5:
            continue
        t = (x - 187.0) / (994.0 - 187.0)
        out.append({"x_px": x, "t": t, "width_px": len(group), "uncertainty_px": 1.5})
    return out

def components_bbox(raw, roi):
    x0, y0, x1, y1 = roi
    pixels = set()
    for y in range(y0, y1):
        for x in range(x0, x1):
            if red_at(raw, x, y):
                pixels.add((x, y))
    result = []
    while pixels:
        first = pixels.pop(); stack = [first]; comp = [first]
        while stack:
            x, y = stack.pop()
            for yy in range(max(y0, y-1), min(y1, y+2)):
                for xx in range(max(x0, x-1), min(x1, x+2)):
                    q = (xx, yy)
                    if q in pixels:
                        pixels.remove(q); stack.append(q); comp.append(q)
        if len(comp) >= 20:
            box = [min(x for x, _ in comp), min(y for _, y in comp), max(x for x, _ in comp), max(y for _, y in comp)]
            comp_set = set(comp)
            boundary = [(x, y) for x, y in comp if any((x+dx, y+dy) not in comp_set for dx, dy in ((1,0),(-1,0),(0,1),(0,-1)))]
            stride = max(1, len(boundary) // 800)
            result.append({"bbox_px": box, "red_pixels": len(comp), "outline_boundary_samples_px": [list(p) for p in sorted(boundary, key=lambda q: (q[1], q[0]))[::stride]], "boundary_uncertainty_px": 1.5})
    return sorted(result, key=lambda item: item["bbox_px"])

def fit_grid(raw):
    def axis(length, cross, origin_guess, period_guess):
        counts = [0] * length
        for y in range(H):
            for x in range(W):
                i = 3 * (y * W + x); r, g, b = raw[i:i+3]
                if max(r, g, b) - min(r, g, b) < 12 and 30 < r < 180:
                    counts[x if cross == "x" else y] += 1
        pairs = []
        for k in range(int((length - origin_guess) / period_guess) + 1):
            predicted = origin_guess + period_guess * k
            lo, hi = max(0, round(predicted)-4), min(length, round(predicted)+5)
            at = max(range(lo, hi), key=lambda idx: counts[idx])
            pairs.append((float(k), float(at), float(counts[at])))
        total = sum(weight for _, _, weight in pairs)
        mk = sum(k*w for k, _, w in pairs) / total
        mx = sum(x*w for _, x, w in pairs) / total
        slope = sum(w*(k-mk)*(x-mx) for k,x,w in pairs) / sum(w*(k-mk)**2 for k,_,w in pairs)
        intercept = mx - slope * mk
        rms = math.sqrt(sum(w*(x-(intercept+slope*k))**2 for k,x,w in pairs) / total)
        return {"origin_px": intercept, "period_px": slope, "detected_lines_px": [round(intercept+slope*k, 2) for k,_,_ in pairs], "rms_px": rms}
    return {"x": axis(W, "x", 34.0, 38.35), "y": axis(H, "y", 3.0, 38.3)}

def green_anchor_markers(raw):
    points = []
    for case, (anchors, _, _) in ANCHORS.items():
        for index, (x, y) in enumerate(anchors):
            if index in (0, len(anchors)-1):
                continue
            count = 0
            for yy in range(max(0, y-2), min(H, y+3)):
                for xx in range(max(0, x-2), min(W, x+3)):
                    i=3*(yy*W+xx); r,g,b=raw[i:i+3]
                    count += g>130 and g>r*1.5 and g>b*1.5
            points.append({"case":case,"anchor_index":index,"pixel": [x,y], "green_pixels_in_5x5":count, "verified":count>0})
    return points

def main():
    raw = decode()
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    grid = fit_grid(raw)
    cases = []
    for case in range(1, 7):
        anchors, pressures, uncertainty = ANCHORS[case]
        profiles = []
        cases.append({
            "id": case,
            "base_width_px": 30 if case == 1 else 8,
            "anchors": [{"pixel": list(point), "pressure_percent": pressure, "marker": ("yellow_endpoint" if i in (0, len(anchors)-1) else "green_interior"), "uncertainty": "±1.5 px"} for i, (point, pressure) in enumerate(zip(anchors, pressures))],
            "reference_centerline_samples_px": [],
            "measured_normal_width_profile": profiles,
            "red_outline_components": components_bbox(raw, ROIS[case]),
            "geometry_reconstruction": "pending validated SaiLine simplified centerline sampling",
            "uncertainty": uncertainty + "; grid/antialiasing contribute approximately ±1.5 px to normal width samples",
        })
    dataset = {"source": "reference.png", "source_sha256": digest, "image_size": [W, H],
        "grid": {**grid, "assumed_grid_spacing_doc_px": 40.0, "inferred_display_scale": grid["x"]["period_px"] / 40.0, "uncertainty_px": 0.8},
        "marker_detection": {"green_interior_anchors": green_anchor_markers(raw), "method": "local 5×5 bright-green pixel check against visually identified centers; endpoints are the yellow markers reviewed in diagnostic.png"},
        "method": "red-pixel masks separated from grayscale grid and green annotations; yellow endpoints and green interior anchors reviewed against the image; widths are sampled along normals to the validated SaiLine simplified centerline",
        "cases": cases}
    (HERE / "dataset.json").write_text(json.dumps(dataset, indent=2) + "\n")

if __name__ == "__main__":
    main()
