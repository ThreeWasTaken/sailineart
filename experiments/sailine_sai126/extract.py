#!/usr/bin/env python3
"""Extract the twelve SAI curves from the fixed reference PNG, using only stdlib + ImageMagick.

Marker order is a reviewed transcription of the visible green/yellow dots, not inferred
from crossings or extrema. The detected centres themselves come from the source pixels.
"""

import hashlib
import heapq
import json
import math
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "reference.png"
W, H = 1306, 874

# Approximate positions only identify the coloured marker component. Pixel centroids are saved.
ORDER = {
    1: [(204, 129), (359, 129)],
    2: [(204, 281), (281, 163), (357, 280)],
    3: [(204, 433), (281, 318), (357, 433), (359, 319)],
    4: [(205, 472), (203, 585), (357, 586), (357, 471)],
    5: [(282, 776), (166, 662), (434, 662), (319, 777)],
    6: [(856, 280), (856, 88), (665, 90), (667, 49), (1126, 49), (1240, 318)],
    7: [(550, 128), (743, 165), (664, 318), (473, 243), (626, 166), (703, 240), (549, 243)],
    8: [(511, 356), (550, 511), (1126, 318)],
    9: [(434, 547), (626, 585), (588, 662), (510, 586), (511, 664)],
    10: [(472, 853), (511, 736), (782, 817), (550, 777)],
    11: [(587, 700), (971, 701), (972, 509), (780, 510), (780, 663)],
    12: [(1087, 471), (1240, 509), (1126, 625), (1277, 852)],
}

# Bounding boxes of the red connected components belonging to each numbered drawing.
# A coloured anchor or blue annotation can split one stroke into several components.
COMPONENT_BOXES = {
    1: [(205, 127, 358, 131)],
    2: [(203, 162, 358, 282)],
    3: [(202, 317, 356, 435), (358, 320, 365, 434)],
    4: [(193, 472, 369, 604)],
    5: [(164, 644, 435, 777), (320, 663, 436, 778)],
    6: [(659, 22, 1241, 317)],
    7: [(472, 127, 748, 323), (550, 165, 705, 249)],
    8: [(510, 317, 1127, 514)],
    9: [(435, 546, 629, 663), (504, 585, 587, 664)],
    10: [(471, 733, 783, 852)],
    11: [(588, 482, 1002, 719)],
    12: [(1125, 510, 1278, 853), (1088, 470, 1241, 508)],
}


def rgb():
    return subprocess.check_output(["magick", str(SOURCE), "-depth", "8", "rgb:-"])


def components(pixels):
    remaining = set(pixels)
    out = []
    while remaining:
        start = remaining.pop()
        stack = [start]
        item = [start]
        while stack:
            x, y = stack.pop()
            for xx in (x - 1, x, x + 1):
                for yy in (y - 1, y, y + 1):
                    q = (xx, yy)
                    if q in remaining:
                        remaining.remove(q)
                        stack.append(q)
                        item.append(q)
        out.append(item)
    return out


def box(pixels):
    return [min(p[0] for p in pixels), min(p[1] for p in pixels),
            max(p[0] for p in pixels), max(p[1] for p in pixels)]


def fit_grid(data):
    # Neutral dark pixels, excluding chromatic strokes. Grid lines are spaced ~38 px.
    xs, ys = [0] * W, [0] * H
    for y in range(H):
        for x in range(W):
            i = 3 * (y * W + x)
            r, g, b = data[i:i + 3]
            if max(r, g, b) - min(r, g, b) < 12 and 30 < r < 180:
                xs[x] += 1
                ys[y] += 1
    # Regression against known lattice indices; intermediate peaks may be double lines.
    def axis(counts, first, step, length):
        pairs = []
        for k in range(length):
            predicted = first + k * step
            choices = range(max(0, round(predicted) - 3), min(len(counts), round(predicted) + 4))
            at = max(choices, key=lambda i: counts[i])
            pairs.append((k, at, counts[at]))
        sw = sum(w for _, _, w in pairs)
        mk = sum(k * w for k, _, w in pairs) / sw
        mx = sum(x * w for _, x, w in pairs) / sw
        slope = sum(w * (k - mk) * (x - mx) for k, x, w in pairs) / sum(w * (k - mk) ** 2 for k, _, w in pairs)
        return {"origin": mx - slope * mk, "period": slope, "detected_lines": [x for _, x, _ in pairs],
                "rms_px": math.sqrt(sum(w * (x - (mx + slope * (k - mk))) ** 2 for k, x, w in pairs) / sw)}
    return {"x": axis(xs, 12, 38.3, 34), "y": axis(ys, 13, 38.3, 23)}


def thin(pixels):
    # Zhang-Suen thinning of the thresholded stroke, then path tracing on 8-neighbour graph.
    p = set(pixels)
    for _ in range(30):
        removed_total = 0
        for pass_no in (0, 1):
            remove = []
            for x, y in p:
                n = [(x, y - 1), (x + 1, y - 1), (x + 1, y), (x + 1, y + 1),
                     (x, y + 1), (x - 1, y + 1), (x - 1, y), (x - 1, y - 1)]
                z = [a in p for a in n]
                count = sum(z)
                if not 2 <= count <= 6 or sum(not z[i] and z[(i + 1) % 8] for i in range(8)) != 1:
                    continue
                if pass_no == 0:
                    valid = not (z[0] and z[2] and z[4]) and not (z[2] and z[4] and z[6])
                else:
                    valid = not (z[0] and z[2] and z[6]) and not (z[0] and z[4] and z[6])
                if valid:
                    remove.append((x, y))
            p.difference_update(remove)
            removed_total += len(remove)
        if not removed_total:
            break
    return p


def nearest(p, point):
    x, y = point
    return min(p, key=lambda q: (q[0] - x) ** 2 + (q[1] - y) ** 2)


def route(pixels, a, b):
    a, b = nearest(pixels, a), nearest(pixels, b)
    queue = [(0.0, a)]
    distance = {a: 0.0}
    parent = {}
    while queue:
        cost, at = heapq.heappop(queue)
        if cost != distance[at]:
            continue
        if at == b:
            path = [b]
            while path[-1] != a:
                path.append(parent[path[-1]])
            return list(reversed(path))
        x, y = at
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                q = (x + dx, y + dy)
                if (dx or dy) and q in pixels:
                    d = cost + (1.41421356237 if dx and dy else 1.0)
                    if d < distance.get(q, math.inf):
                        distance[q], parent[q] = d, at
                        heapq.heappush(queue, (d, q))
    return []


def svg_diagnostic(cases, grid):
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">',
             '<rect width="100%" height="100%" fill="#fff"/>']
    for x in grid["x"]["detected_lines"]:
        parts.append(f'<path d="M{x},0 V{H}" stroke="#ddd"/>')
    for y in grid["y"]["detected_lines"]:
        parts.append(f'<path d="M0,{y} H{W}" stroke="#ddd"/>')
    colours = ["#bf1759", "#0c81bb", "#1d9b50", "#8e48b8", "#d87500", "#169c9c",
               "#b53822", "#274acf", "#968000", "#d04b91", "#216e70", "#8158bc"]
    for case in cases:
        colour = colours[case["id"] - 1]
        for seg in case["reference_segments_px"]:
            if len(seg) > 1:
                coords = " ".join(f'{x:.1f},{y:.1f}' for x, y in seg)
                parts.append(f'<polyline points="{coords}" fill="none" stroke="{colour}" stroke-width="2.5"/>')
        for i, anchor in enumerate(case["anchors"]):
            x, y = anchor["pixel"]
            parts.append(f'<circle cx="{x}" cy="{y}" r="4" fill="white" stroke="{colour}" stroke-width="1.5"/>')
            parts.append(f'<text x="{x+5}" y="{y-5}" font-size="11" fill="#111">{case["id"]}.{i}</text>')
    parts.append('</svg>')
    (HERE / "diagnostic.svg").write_text("\n".join(parts))
    subprocess.run(["magick", "-background", "white", str(HERE / "diagnostic.svg"), str(HERE / "diagnostic.png")], check=True)


def main():
    data = rgb()
    if len(data) != W * H * 3:
        raise ValueError("reference dimensions changed")
    grid = fit_grid(data)
    red, marks = set(), set()
    for y in range(H):
        for x in range(W):
            i = 3 * (y * W + x)
            r, g, b = data[i:i + 3]
            if r > 180 and r > g * 1.8 and r > b * 1.8:
                red.add((x, y))
            if (r > 240 and 160 < g < 245 and b < 80) or (g > 180 and g > r * 1.5 and g > b * 1.5):
                marks.add((x, y))
    mark_components = [c for c in components(marks) if 3 <= len(c) <= 10]
    markers = [(sum(x for x, _ in c) / len(c), sum(y for _, y in c) / len(c),
                "green" if data[3 * (round(c[0][1]) * W + round(c[0][0])) + 1] > 245 else "yellow")
               for c in mark_components]
    if len(markers) != 51:
        raise ValueError(f"expected 51 coloured markers, got {len(markers)}")
    red_components = [c for c in components(red) if len(c) >= 100]
    assigned = {}
    for component in red_components:
        actual = box(component)
        matches = [number for number, boxes in COMPONENT_BOXES.items()
                   if any(max(abs(a - b) for a, b in zip(actual, expected)) <= 2 for expected in boxes)]
        if len(matches) != 1:
            raise ValueError(f"red component {actual} has ambiguous assignment {matches}")
        assigned.setdefault(matches[0], set()).update(component)
    if len(red_components) != sum(len(x) for x in COMPONENT_BOXES.values()):
        raise ValueError(f"unexpected red component count {len(red_components)}")
    cases = []
    used_markers = set()
    for number, rough_order in ORDER.items():
        anchors = []
        for rough in rough_order:
            nearest_id = min(range(len(markers)), key=lambda j: (markers[j][0] - rough[0]) ** 2 + (markers[j][1] - rough[1]) ** 2)
            x, y, colour = markers[nearest_id]
            if nearest_id in used_markers or math.dist((x, y), rough) > 3:
                raise ValueError(f"marker assignment failed for {number}: {rough}")
            used_markers.add(nearest_id)
            anchors.append({"pixel": [x, y], "grid": [(x - grid["x"]["origin"]) / grid["x"]["period"],
                                                    (y - grid["y"]["origin"]) / grid["y"]["period"]],
                            "colour": colour, "uncertainty_px": 1.0})
        case_red = assigned[number]
        # Fill the coloured 5-pixel star that replaces the red line at each anchor.
        mask = set(case_red)
        for anchor in anchors:
            x, y = (round(z) for z in anchor["pixel"])
            mask.update((xx, yy) for xx in range(x - 2, x + 3) for yy in range(y - 2, y + 3)
                        if (xx - x) ** 2 + (yy - y) ** 2 <= 5)
        skeleton = thin(mask)
        segments = []
        uncertainty = []
        for a, b in zip(anchors, anchors[1:]):
            path = route(skeleton, a["pixel"], b["pixel"])
            if not path:
                uncertainty.append("No connected skeleton route between two markers")
                path = []
            elif math.dist(path[0], a["pixel"]) > 3 or math.dist(path[-1], b["pixel"]) > 3:
                uncertainty.append("Skeleton route endpoint exceeds 3 px from marker")
            segments.append([[float(x), float(y)] for x, y in path])
        cases.append({"id": number, "anchors": anchors, "reference_segments_px": segments,
                      "reference_segments_grid": [[[ (x - grid["x"]["origin"]) / grid["x"]["period"],
                                                      (y - grid["y"]["origin"]) / grid["y"]["period"]]
                                                     for x, y in segment] for segment in segments],
                      "red_pixel_count": len(case_red), "skeleton_pixel_count": len(skeleton),
                      "uncertainty": uncertainty,
                      "reference_uncertainty_px": 1.5,
                      "marker_order": "manual review of coloured markers along red stroke; orientation arbitrary"})
    if len(used_markers) != len(markers):
        raise ValueError(f"{len(markers) - len(used_markers)} marker(s) unassigned")
    output = {"source": str(SOURCE.name), "sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              "image_size": [W, H], "grid": grid, "method": "red threshold, component masks, Zhang-Suen skeleton, shortest skeleton route between reviewed markers",
              "cases": cases}
    (HERE / "dataset.json").write_text(json.dumps(output, indent=2) + "\n")
    svg_diagnostic(cases, grid)
    print("grid", grid["x"]["origin"], grid["x"]["period"], grid["y"]["origin"], grid["y"]["period"])
    for case in cases:
        print(case["id"], "anchors", len(case["anchors"]), "routes", [len(p) for p in case["reference_segments_px"]], case["uncertainty"])


if __name__ == "__main__":
    main()
