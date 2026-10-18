#!/usr/bin/env python3
"""Render marker/normal diagnostics and a full six-case SAI/PhotoCraft silhouette overlay."""
import base64, json, math, subprocess
from pathlib import Path
HERE=Path(__file__).resolve().parent
D=json.loads((HERE/'dataset.json').read_text())
G=json.loads((HERE/'geometry_samples.json').read_text())
R=json.loads((HERE/'results.json').read_text())
image=base64.b64encode((HERE/'reference.png').read_bytes()).decode()

def esc(s): return s.replace('&','&amp;').replace('<','&lt;')
# Extraction diagnostic: cyan normals, cyan observed endpoints, green detected interior controls.
parts=[f'<svg xmlns="http://www.w3.org/2000/svg" width="1275" height="859"><image width="1275" height="859" href="data:image/png;base64,{image}"/>']
parts.append('<g fill="none" stroke="#00d9ff" stroke-width="2">')
for case in D['cases']:
    for anchor in case['anchors']:
        x,y=anchor['pixel'];color='#00e879' if anchor['marker']=='green_interior' else '#00d9ff'
        parts.append(f'<circle cx="{x}" cy="{y}" r="6" stroke="{color}"/>')
    for row in G['cases'][case['id']-1]['samples'][::20]:
        x,y=row['center_px'];nx,ny=row['normal_px']
        parts.append(f'<path d="M {x-6*nx:.2f} {y-6*ny:.2f} L {x+6*nx:.2f} {y+6*ny:.2f}" stroke="#00ffff" stroke-width="1"/>')
parts.append('</g><rect x="12" y="8" width="800" height="27" fill="white" fill-opacity=".82"/><text x="20" y="28" font-family="sans-serif" font-size="15" fill="#111">Detected markers: green interior, cyan yellow-endpoint centers · cyan ticks: sampling normals</text></svg>')
(HERE/'diagnostic.svg').write_text(''.join(parts))
subprocess.run(['magick',str(HERE/'diagnostic.svg'),str(HERE/'diagnostic.png')],check=True)

# Blue translucent fills are the predicted normal-offset outlines using linear pressure by arc length.
# The underlying red/green screenshot is the SAI reference; overlap appears purple.
board=[f'<svg xmlns="http://www.w3.org/2000/svg" width="1275" height="859"><image width="1275" height="859" href="data:image/png;base64,{image}"/>','<g fill="#148aff" fill-opacity=".34" stroke="#006be6" stroke-opacity=".9" stroke-width="1.1">']
scale=D['grid']['inferred_display_scale']
for case in D['cases']:
    rows=G['cases'][case['id']-1]['samples']
    for seg in range(len(case['anchors'])-1):
        part=[r for r in rows if r['segment_index']==seg]
        if len(part)<2: continue
        p0=case['anchors'][seg]['pressure_percent']/100
        p1=case['anchors'][seg+1]['pressure_percent']/100
        left=[];right=[]
        for row in part:
            u=row['segment_arc_fraction'];pressure=max(0,p0+(p1-p0)*u)
            radius=case['base_width_px']*scale*pressure*.5
            x,y=row['center_px'];nx,ny=row['normal_px']
            left.append((x+nx*radius,y+ny*radius));right.append((x-nx*radius,y-ny*radius))
        pts=left+right[::-1]
        if pts:
            d='M '+' L '.join(f'{x:.2f} {y:.2f}' for x,y in pts)+' Z'
            board.append(f'<path d="{d}"/>')
        for anchor_index in [seg,seg+1]:
            if anchor_index not in (0,len(case['anchors'])-1): continue
            a=case['anchors'][anchor_index];radius=case['base_width_px']*scale*a['pressure_percent']/200
            if radius>0:
                x,y=a['pixel'];board.append(f'<circle cx="{x}" cy="{y}" r="{radius:.2f}"/>')
board.append('</g><rect x="12" y="8" width="910" height="30" fill="white" fill-opacity=".86"/><text x="20" y="29" font-family="sans-serif" font-size="17" font-weight="bold" fill="#111">Sai (red) vs predicted width outline (blue): linear pressure by arc length · overlap is purple</text></svg>')
(HERE/'comparison_board.svg').write_text(''.join(board))
subprocess.run(['magick',str(HERE/'comparison_board.svg'),str(HERE/'comparison_board.png')],check=True)
