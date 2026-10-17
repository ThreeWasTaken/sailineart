# SaiLine

SaiLine creates an open interpolated vector stroke through a series of clicked points.
Select **SaiLine Tool** as its own toolbar slot beside the Pen tools.

Its Bézier handles respond to neighboring segment lengths: longer spans pull a wider arc,
shorter turns stay tighter, and nearly coincident anchors converge toward a sharp corner.
The extra pull fades continuously as neighboring segments approach equal length.
On a long-to-short turn, the outgoing handle follows the short segment more closely while keeping
its short handle length; the incoming handle keeps the preceding sweep broad. A shorter final
return continues a gentle arc, while a long final span retains its natural endpoint tangent.
The endpoint correction fades in with the length difference and fades out near a right-angle
turn. Near an opposing turn, the outgoing handle contracts continuously toward zero.
For a three-point arc with comparable neighboring spans, the free start and end continue the
turn gently so the curve bows toward the outside from both endpoints.

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
