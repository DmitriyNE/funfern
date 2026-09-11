# Browser interaction checks

Status: the WebGPU binding-limit startup failure found on 2026-09-09 was fixed by
repacking the wave compute layout into eight bindings. The user subsequently
confirmed that the browser build runs. This checklist reflects the current shell;
repeat it after interaction or rendering changes.

Start `trunk serve --release`, then open <http://127.0.0.1:8080/>.

- [ ] Initial view fills the window with the square, rounded loop, grid, control
  polygon and handles. The top bar, contextual right inspector, and bottom status
  strip stay crisp and correctly placed through resize and display-scale changes.
- [ ] Hide every inspector panel and restore each of Edit, View, Simulation, and
  Materials. The viewport expands when the inspector is hidden. At a narrow width,
  File and panel menus replace buttons without obscuring playback controls.
- [ ] Draw > Hole/Interface > Circle creates one loop and ends placement. Custom
  previews after four controls; exercise Finish, Enter, first-handle closure,
  Backspace, and Escape. Draw > Baffle offers Straight and Custom. The viewport
  overlay always states the active mode and provides Cancel or Done.
- [ ] Drag a handle outside the square. The invalid draft persists after release,
  is red, and shows a specific reason and the accepted reference. Drag it back
  to recover. Escape during a drag restores both scenes. Undo restores the prior
  document snapshot.
- [ ] Double-click a curve to insert, including near the seam. The shape stays
  fixed. Double-click the same knot again: select its control without insertion.
  Delete removes a control, down to four. Delete obstacle is separate.
- [ ] Select one control handle, one span, several spans, and a whole curve. Test
  Shift-toggle, Command/Ctrl-click whole curve, marquee replace, Shift-add,
  Alt-subtract, filters, Select filtered, Invert, and Clear. Mixed-object span
  selection must not expose a whole-object delete or role change.
- [ ] Drag selected spans as one rigid piece with snapping enabled. Exercise the
  rotation ring, square scale grip, Shift angle/scale snapping, movable transform
  center, and numeric translate/rotate/scale controls. With persistent snapping
  off, Shift-drag a selected and an unselected span and verify grid snapping without
  losing the selection. Verify that no transform submode is required.
- [ ] Assign one condition to multiple outer, hole, and baffle spans. On baffles,
  verify coherent left/right face selection and that Thin gap replaces the two
  independent face conditions instead of competing with them. Enable View >
  Boundary conditions and compare the canvas colors with the legend.
- [ ] Use C0 isolation, continuity downgrade/reshape, baffle straightening,
  splitting, and endpoint merging. Endpoint-only actions appear only for a valid
  endpoint selection; merge is enabled only for two complete baffles within the
  join distance.
- [ ] Undo/redo each action through buttons and Ctrl/Cmd shortcuts. One drag or
  completed coordinate edit is one action. Undo/redo restores invalid drafts.
- [ ] Type coordinates. Delete, Space, and Ctrl/Cmd+Z while typing are captured
  by text editing. Scroll the right inspector without moving or zooming geometry.
- [ ] Right-drag and Space+left-drag pan. Wheel zoom preserves the world point
  under the cursor. A drag that starts on the panel never edits geometry.
- [ ] Resize the window and change display scale. The box stays square, controls
  stay aligned, and Fit View centers the fixed domain.
- [ ] Save a valid scene and a scene with an invalid draft. Reload both using
  file upload. IDs and nonuniform intervals survive. History clears on load.
  Cancel dialogs and try malformed JSON, duplicate IDs, unsupported versions,
  oversized files, and an invalid accepted scene; the document remains intact.
- [ ] In Simulation, toggle Place pulse and Move source on and off by clicking the
  selected button. Both modes persist across repeated viewport clicks and inspector
  changes until explicitly ended. Verify the pulse footprint and source marker.
  Run/Pause and Reset remain at the right of the top bar; Step is available while
  paused. Energy is visible in Simulation and Performance diagnostics.
- [ ] Add, name, edit, assign, and delete a material. Delete is disabled while the
  material is assigned. Region rows and canvas material colors remain consistent.
- [ ] Open Performance from the lower-right summary. Verify FPS, steps/s, DOFs,
  mesh size, solver dt, energy, frame-time graph/statistics, mesh quality, handoff,
  and solver sections. A normal rebuild must not open diagnostics or show a warning;
  an injected or real mesh/solver error should do both.
- [ ] Disable WebGPU or use an unsupported browser and reload. Readable startup
  guidance must remain visible. Also check an adapter/device initialization failure.
- [ ] Load `examples/eight-obstacles.json`. Observe idle and dragging frame times,
  validation latency, smoothness, and browser console errors. Try 32 obstacles.

Keyboard focus, hover help, mode-specific cursors, contrast, and panel traversal
receive practical accessibility coverage. The numerical canvas remains a custom
drawn surface and is not represented as a screen-reader geometry tree.

Record browser version, OS, GPU/backend from the Bevy console, canvas size/device
pixel ratio, release/debug mode, idle/drag frame-time ranges, and any observed
validation delay in `engineering-log.md`. The status strip reports a smoothed
whole-frame interval, not isolated GPU execution time. Do not infer browser
performance from the native startup check or headless egui tests.
