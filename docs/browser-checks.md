# Milestone 1 browser checks

Status: the first actual browser launch on 2026-09-09 rendered one frame and then
stopped processing input and resize events. The wave compute layout exceeded the
portable WebGPU limit with ten storage buffers in one shader stage; it has been
repacked into eight bindings. Native GPU execution and a release WASM build pass.
The post-fix interactive browser pass and browser/GPU metadata remain pending.

Start `trunk serve --release`, then open <http://127.0.0.1:8080/>.

- [ ] Initial view fills the window with the square, rounded loop, grid, polygon,
  handles, left panel, and status strip; simulation controls are disabled.
- [ ] Rounded mode creates a loop at the click and returns to Select. Custom
  mode previews after four controls; exercise Enter, first-handle closure,
  Backspace, and Escape.
- [ ] Drag a handle outside the square. The invalid draft persists after release,
  is red, and shows a specific reason and the accepted reference. Drag it back
  to recover. Escape during a drag restores both scenes. Revert Draft works.
- [ ] Double-click a curve to insert, including near the seam. The shape stays
  fixed. Double-click the same knot again: select its control without insertion.
  Delete removes a control, down to four. Delete obstacle is separate.
- [ ] Undo/redo each action through buttons and Ctrl/Cmd shortcuts. One drag or
  completed coordinate edit is one action. Undo/redo restores invalid drafts.
- [ ] Type coordinates. Delete, Space, and Ctrl/Cmd+Z while typing are captured
  by text editing. Scroll the left panel without moving or zooming geometry.
- [ ] Middle-drag and Space+left-drag pan. Wheel zoom preserves the world point
  under the cursor. A drag that starts on the panel never edits geometry.
- [ ] Resize the window and change display scale. The box stays square, controls
  stay aligned, and Fit View centers the fixed domain.
- [ ] Save a valid scene and a scene with an invalid draft. Reload both using
  file upload. IDs and nonuniform intervals survive. History clears on load.
  Cancel dialogs and try malformed JSON, duplicate IDs, unsupported versions,
  oversized files, and an invalid accepted scene; the document remains intact.
- [ ] Disable WebGPU / use an unsupported browser and reload. Readable startup
  guidance must remain visible. Also check an adapter/device initialization failure.
- [ ] Load `examples/eight-obstacles.json`. Observe idle and dragging frame times,
  validation latency, smoothness, and browser console errors. Try 32 obstacles.

Record browser version, OS, GPU/backend from the Bevy console, canvas size/device
pixel ratio, release/debug mode, idle/drag frame-time ranges, and any observed
validation delay in `engineering-log.md`. The status strip reports a smoothed
whole-frame interval, not isolated GPU execution time. Do not infer browser
performance from the native startup check or headless egui tests.
