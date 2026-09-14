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
- [ ] Draw offers Closed curve with Circle, Rectangle, Polygon, and Spline under a
  Subdomain or Hole purpose, and Open curve with Polyline and Spline under a
  Separator or Baffle purpose. Exercise two-corner Rectangle, vertex-based Polygon
  with Enter and first-vertex closure, and the four-control Spline preview. Finish
  an open Spline at exactly two controls and verify an exact straight baffle;
  verify that three controls cannot finish and four or more retain the freeform
  spline behavior. Check Finish, Enter, Backspace, Escape, previews, one-entry
  history, invalid drafts, and the 128-control limit. The viewport overlay
  identifies vertices versus control points.
- [ ] While drawing an open curve, confirm that eligible outer edges, curve
  interiors, junctions, loose ends, and vertex-less corners all highlight, and that
  starting or finishing on a loose end welds the new curve into that one. A
  separator must start and finish on the same active face.
- [ ] Drag a handle outside the square. The invalid draft persists after release,
  is red, and shows a specific reason and the accepted reference. Drag it back
  to recover. Escape during a drag restores both scenes. Undo restores the prior
  document snapshot.
- [ ] Double-click a curve to insert, including near the seam. The shape stays
  fixed. Double-click the same knot again: select its control without insertion.
  Delete removes a control, down to four. Delete obstacle is separate.
- [ ] Select one control handle, one span, several spans, and a whole curve. Test
  Shift-toggle, Command/Ctrl-click whole curve, left-to-right fully-enclosed
  marquee, right-to-left crossing marquee, Shift-add, Alt-subtract, filters,
  Select filtered, Invert, and Clear. Press and release Shift or Alt while a marquee
  is already moving and confirm that its operation and result update immediately.
  Exercise persistent Area select with Replace, Add, and Subtract. Selecting one
  span must ring only that span's own four controls, not the whole curve.
- [ ] Press Delete and Backspace with a control, a probe, one complete feature, and
  several complete features selected, including while the pointer is over a panel.
  A multi-feature deletion is one undo entry; partial span and outer-edge selections
  remain present.
- [ ] Drag selected spans as one rigid piece with snapping enabled. Exercise the
  rotation ring, square scale grip, Shift angle/scale snapping, movable transform
  center, and numeric translate/rotate/scale controls. With persistent snapping
  off, Shift-drag a selected and an unselected span and verify grid snapping without
  losing the selection. Verify that no transform submode is required.
- [ ] Assign one condition to multiple outer, hole, and baffle spans. On baffles,
  verify coherent left/right face selection and that Thin gap replaces the two
  independent face conditions instead of competing with them. Enable View >
  Boundary conditions and compare the canvas colors with the legend.
- [ ] Use C0 isolation, continuity promotion and downgrade, and both straightening
  actions on a complete baffle and a partial C0-bounded loop piece. **Straighten
  spans** should isolate and straighten every selected span independently;
  **Straighten selection** should make each contiguous C0-bounded run one chord.
  Both preserve the relevant endpoints, conditions, and probe attachments in one
  undo entry. Promote the seam of a two-span loop to C2 and confirm that the curve
  gains the spans it needs, does not jump, and comes back in one undo. A junction
  must keep its C0 buttons disabled.
- [ ] Weld by dragging. Drop a loose end on another loose end, on its own other
  end, on a junction, on a curve interior, on a vertex-less corner, and on its own
  curve. Each is one undo entry, the seam is promotable, and a drop the arrangement
  rejects leaves only the drag. Dragging a single-span curve onto itself should
  refine it rather than refuse.
- [ ] Delete spans that merge two subdomains. The candidates highlight in gold with
  their material named, a click picks the survivor, and Escape cancels. A span with
  a hole on both sides reads Inactive in the Edit panel.
- [ ] In Materials, switch Subdomain assignment between Faces and Regions. Faces
  lists holes as well; turn a subdomain into a hole and back and confirm that only
  that face's own boundary changes. A click in the viewport selects a face or a
  region to match the toggle.
- [ ] Resize the outer rectangle by dragging a side and a corner grip, with Shift
  snapping to the grid. Each drag is one undo entry. Confirm the resize cursors
  appear on hover, before the drag starts, along with the gizmo cursors.
- [ ] Undo/redo each action through buttons and Ctrl/Cmd shortcuts. One drag or
  completed coordinate edit is one action. Undo/redo restores invalid drafts.
- [ ] Type coordinates. Delete, Space, and Ctrl/Cmd+Z while typing are captured
  by text editing. Scroll the right inspector without moving or zooming geometry.
- [ ] Right-drag and Space+left-drag pan. Wheel zoom preserves the world point
  under the cursor. A drag that starts on the panel never edits geometry.
- [ ] On a touch device, tap-select and drag handles, curves, sources, probes,
  domain edges, and transform gizmos. Empty one-finger drag draws a directional
  marquee; two fingers pan and pinch around their centroid. Add a second finger
  during an object drag or marquee and verify that the tentative action rolls back
  without a history entry or release click. Confirm that placement tools accept
  repeated taps, Area select works in both directions, panel scrolling does not move
  geometry, and touch targets are comfortable without visibly enlarged handles.
- [ ] Resize the window and change display scale. The box stays square, controls
  stay aligned, and Fit View centers the current domain.
- [ ] Save a valid scene and a scene with an invalid draft. Reload both using
  file upload. IDs and nonuniform intervals survive. History clears on load.
  Cancel dialogs and try malformed JSON, duplicate IDs, unsupported versions,
  oversized files, and an invalid accepted scene; the document remains intact.
- [ ] Export a viewport PNG with the inspector open, floating probe/performance
  windows open, an active selection, and several View overlays enabled. The download
  should contain only the central viewport at its physical pixel dimensions: active
  View overlays and the logo remain, while menus, panels, readouts, selection
  emphasis, gizmos, marquees, and tool prompts are absent. Repeat after resize and
  at a non-1× display scale; capture must not pause playback or alter camera,
  document, presentation, selection, or history.
- [ ] Start **Record viewport** from the Export menu while the simulation is
  running. The first video frame must not contain the menu. Confirm that the field,
  active View overlays, probes, and logo remain while panels, floating readouts,
  selections, gizmos, marquees, prompts, and the red recording/status controls stay
  outside the video. Pan and zoom, resize the browser, pause and resume, then stop
  from the status bar. The downloaded WebM or MP4 should retain its original frame
  dimensions, letterbox rather than stretch after resize, play for the elapsed
  wall-clock duration, and contain no audio. Repeat once while initially paused and
  record the chosen MIME type from the status tooltip. An unsupported MediaRecorder
  or canvas-stream implementation should produce a readable error and restore the
  ordinary viewport.
- [ ] In Simulation, toggle Place pulse on and off by clicking the selected button;
  it persists across repeated viewport clicks and inspector changes until explicitly
  ended. Enable the continuous source and drag its marker directly. Verify that a
  saved file, copied scene link, and autosave restore its position and parameters.
  Run/Pause and Reset remain at the right of the top bar; Step is available while
  paused. Energy is visible in Simulation and Performance diagnostics.
- [ ] Add, name, edit, assign, and delete a material. Delete is disabled while the
  material is assigned. Region rows and canvas material colors remain consistent.
- [ ] Open Performance from the lower-right summary. Verify FPS, steps/s, DOFs,
  mesh size, solver dt, energy, frame-time graph/statistics, mesh quality, handoff,
  and solver sections. After moving a hole and a material interface, verify local
  attempt count, retry causes, repair vertices/triangles, moved/inserted/collapsed
  vertices, reuse percentage, and the session fallback histogram. During a retry,
  the status should read `Mesh rebuilding: Expanding local repair`. Move, rotate,
  scale, straighten, and locally reshape a baffle; verify a local
  result, nonzero repaired-baffle/paired-segment counts, and a successful solver
  handoff. A normal rebuild must not open diagnostics or show a warning; an injected
  or real mesh/solver error should do both.
- [ ] Disable WebGPU or use an unsupported browser and reload. Readable startup
  guidance must remain visible. Also check an adapter/device initialization failure.
- [ ] Load `examples/eight-obstacles.json`, or Obstacle array from the gallery,
  which is the same scene. Observe idle and dragging frame times, validation
  latency, smoothness, and browser console errors. Try 32 obstacles.
- [ ] Enable View > Mesh over a running field and confirm the wireframe is visible
  on top of it, with and without a material overlay. With Mesh off, neither the
  Materials nor the Subdomains overlay should print the triangulation.

Keyboard focus, hover help, mode-specific cursors, contrast, and panel traversal
receive practical accessibility coverage. The numerical canvas remains a custom
drawn surface and is not represented as a screen-reader geometry tree.

Record browser version, OS, GPU/backend from the Bevy console, canvas size/device
pixel ratio, release/debug mode, idle/drag frame-time ranges, and any observed
validation delay in `engineering-log.md`. The status strip reports a smoothed
whole-frame interval, not isolated GPU execution time. Do not infer browser
performance from the native startup check or headless egui tests.
