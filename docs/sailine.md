# SaiLine

SaiLine creates an open interpolated vector stroke through a series of clicked points.
Select **SaiLine Tool** as its own toolbar slot beside the Pen tools.

- Click the canvas to place anchors. Moving the pointer previews the next point.
- Around the last anchor, the pointer also controls the curve: its direction changes the tangent
  angle, while its distance changes how rounded the preceding segment is.
- Choose the constant width and foreground color in the options bar.
- Press Enter, click Done, or right-click the canvas to create a dedicated linework layer, in one
  undoable history step.
- Press Escape or click Cancel to discard the draft. Backspace removes its last point; Ctrl+Z and
  Ctrl+Y undo and restore draft points without changing document history.
- Leaving the tool or switching documents discards the draft.
- On an existing linework layer, click a point to select it; Shift-click adds or removes points
  from the selection. Ctrl-drag any selected point moves the selected points together, with one
  undo step for the complete drag. Hovering a point highlights it before selection.
- Hold `Shift` and drag from an empty area to draw a temporary selection rectangle around points.
- With a point selected, use **Point width** in the SaiLine options to change its width factor;
  the outline updates immediately and the change is undoable.
- With a point selected, **Path color ← foreground** applies the current foreground color to the
  whole path; the operation is undoable and persists with the path.

At least two points are required; a stroke accepts at most 4096 points. The draft does not
change the document until it is finished. Native `.pcraft` saves keep the canonical point list,
path IDs and point IDs in the dedicated `LineworkLayer` format.

This milestone supports open paths with interpolated per-point width, point insertion/deletion,
and multi-point translation. Split/join and global curvature controls remain future work.
The engine command is `linework.create`; it is also available through the CLI and automation.
