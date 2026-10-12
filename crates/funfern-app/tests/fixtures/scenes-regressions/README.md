# Regression scenes

One file per scene that failed in use, saved as the app saved it and named
by date and defect. They stand for what a user had open when something
broke, and `tests/regression_scenes.rs` prepares each the way the app does,
at the file's own mesh edge, with the frozen version 22 scenes and the
gallery beside them.

Never edit or regenerate these. A reported bug that comes with a scene adds
its file here in the fix's commit; a schema change keeps every file opening,
by a serde default or by a migration under a new version.

- `2026-10-12-one-sided-arms-beside-a-line.json`: a U outgoing along the
  inside of its arms and a line between them outgoing along one side, the
  arms' ends 0.05 from the line's. The banded trace eigensolve wrote a zero
  the mesh pattern holds between two pieces of the trace over an arm's
  diagonal and refused the scene as not positive semidefinite (fixed in
  9d19e20, `docs/engineering-log.md`).
