# Version 22 scenes written before a rule they break

Version 22 files that a rule added since refuses at acceptance. The format has
not changed, so each is what a build before the rule wrote, and stands for the
files, links and autosaves users made then.

- `driven-span-meeting-a-pinned-one.json`: `scenes-v22/drum-modes.json` with
  its curve's first span driven, a 1.5 Hz harmonic, where the other spans pin
  the membrane. Two Dirichlet conditions with different signals meet on one
  node, which acceptance refuses since e6ffc06 (2026-10-12) and assembly
  refused before it.

Each must open with every value it held read back unchanged, its accepted scene
kept as written, and its draft invalid, naming the rule, until it is edited
(`tests/frozen_scenes.rs`, `tests/regression_scenes.rs`; PER-2 of the
2026-10-20 review). Never edit or regenerate these; add a scene for a new rule.
