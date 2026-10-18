#!/usr/bin/env python3
"""Measure the red stroke's cross-section on normals emitted by PhotoCraft's Rust geometry."""
import json
import math
import subprocess
from pathlib import Path
HERE=Path(__file__).resolve().parent
W,H=1275,859
raw=subprocess.check_output(['magick',str(HERE/'reference.png'),'-depth','8','RGB:-'])
geometry=json.loads((HERE/'geometry_samples.json').read_text())
dataset=json.loads((HERE/'dataset.json').read_text())
def red(x,y):
    x=round(x);y=round(y)
    if not (0<=x<W and 0<=y<H): return False
    i=3*(y*W+x);r,g,b=raw[i:i+3]
    return r>170 and g<125 and b<125 and r>g*1.25
byid={c['id']:c for c in geometry['cases']}
for case in dataset['cases']:
    geom=byid[case['id']]['samples']
    centers=[g['center_px'] for g in geom]
    profile=[];centerline=[]
    scale=dataset['grid']['inferred_display_scale']
    limit=case['base_width_px']*scale+5
    for index,g in enumerate(geom):
        x,y=g['center_px'];nx,ny=g['normal_px'];seg=g['segment_index'];t=g['t']
        centerline.append([x,y])
        if math.hypot(nx,ny)<0.8 or t<0.015 or t>0.985: continue
        # Mark a section ambiguous when a non-adjacent centerline arm enters the maximum stroke diameter.
        other=min((math.hypot(x-q[0],y-q[1]) for j,q in enumerate(centers)
                   if abs(j-index)>5 and abs(geom[j]['segment_index']-seg)>1),default=1e9)
        ambiguous=other < 2*limit+2
        offsets=[-limit + k*0.5 for k in range(int(2*limit/0.5)+1)]
        hits=[s for s in offsets if red(x+nx*s,y+ny*s)]
        if not hits: continue
        # Grid lines can mask a few pixels; use outer red support on each side of the fitted axis.
        neg=[s for s in hits if s<0];pos=[s for s in hits if s>0]
        width=(max(pos) if pos else 0.0)-(min(neg) if neg else 0.0)
        profile.append({"segment_index":seg,"t":t,"segment_arc_fraction":g['segment_arc_fraction'],"path_arc_fraction":g['path_arc_fraction'],"segment_length_px":g['segment_length_px'],"center_px":[x,y],"normal_px":[nx,ny],"width_px":width,"uncertainty_px":1.5,"ambiguous":ambiguous})
    case['reference_centerline_samples_px']=centerline
    case['measured_normal_width_profile']=profile
    case['normal_measurement_summary']={"samples":len(profile),"ambiguous_samples":sum(p['ambiguous'] for p in profile),"method":"red mask extent along the normal to photocraft-vector SaiSimplified centerline; half-pixel sampling; ±1.5 px extraction uncertainty"}
    print(f"case {case['id']}: {len(profile)} width samples ({sum(p['ambiguous'] for p in profile)} ambiguous)")
(HERE/'dataset.json').write_text(json.dumps(dataset,indent=2)+'\n')
