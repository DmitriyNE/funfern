# Frozen version 22 scenes

Every gallery scene as version 22 wrote it on 2026-10-03, in compact JSON as a
shared link carries it, and one shared-link fragment (`brewster-angle.link`).
They stand for the files, links and autosaves users made with version 22.

Never edit or regenerate these. `tests/frozen_scenes.rs` and the link test in
`src/sharing.rs` require each to open with every value it held read back
unchanged. A change to the schema keeps them opening, by a serde default or by
a migration under a new version; a new version adds its own frozen set beside
this one.
