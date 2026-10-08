//! The Materials panel's editing session: which material is open, the copy
//! of it staged for Apply, and what is being typed or picked into its fields
//! before it is committed.
//!
//! Each of these belongs to the document as it stood when the editing began,
//! and each has been found outliving it: formula text for laws a preset had
//! replaced, a staged copy written back over a converted material, text typed
//! for one scene's material landing on the next scene's material of the same
//! id. So what happens to them all when the document moves is decided here,
//! one method to a transition, rather than at each place that moves it.

use super::*;

pub(super) struct MaterialSession {
    /// The material the panel has open.
    pub(super) selection: MaterialId,
    /// The open material as staged for Apply.
    pub(super) staged: Option<Material>,
    /// Text typed into formula fields not yet committed, keyed by material
    /// and slot, and the error each field shows. A volume source's profile
    /// keeps its text here too, under its region's key.
    pub(super) formulas: BTreeMap<(u64, u8), String>,
    pub(super) errors: BTreeMap<(u64, u8), String>,
    /// A parameter name being typed, keyed by material and position, with the
    /// name it was opened on.
    pub(super) names: BTreeMap<(u64, usize), (String, String)>,
    /// A colour being picked, committed once the pointer lets go.
    pub(super) colour: Option<(MaterialId, [u8; 3])>,
}

impl Default for MaterialSession {
    fn default() -> Self {
        Self {
            selection: DEFAULT_MATERIAL,
            staged: None,
            formulas: BTreeMap::new(),
            errors: BTreeMap::new(),
            names: BTreeMap::new(),
            colour: None,
        }
    }
}

impl MaterialSession {
    /// Opens `material` from the Library: nothing staged or typed for the
    /// one open before carries over.
    pub(super) fn open(&mut self, material: MaterialId) {
        self.selection = material;
        self.reload();
    }

    /// The open material is read from the document again, and what was
    /// typed or picked against the one it replaces goes: after an undo or a
    /// redo, when a material is opened, and after a removal that may have
    /// taken what the material was for. A parameter name being typed went on
    /// to show when its material next opened, over the name the material
    /// has, and a colour being dragged when Cmd+Z stepped the history landed
    /// on the step it came to, or, its material undone, on the material a
    /// Redo brought back, as an edit that cleared the rest of the Redo.
    pub(super) fn reload(&mut self) {
        self.staged = None;
        self.formulas.clear();
        self.errors.clear();
        self.names.clear();
        self.colour = None;
    }

    /// A whole scene came in: nothing of the outgoing scene's materials
    /// carries over, since the incoming one numbers its materials from the
    /// same small ids, and the default material opens.
    pub(super) fn scene_replaced(&mut self) {
        self.selection = DEFAULT_MATERIAL;
        self.reload();
    }

    /// Drops the formula, error and parameter-name text of one material, whose
    /// laws a preset or Revert has just replaced.
    pub(super) fn forget(&mut self, material: MaterialId) {
        self.formulas.retain(|(owner, _), _| *owner != material.0);
        self.errors.retain(|(owner, _), _| *owner != material.0);
        self.names.retain(|(owner, _), _| *owner != material.0);
    }

    /// The staged copy through the conversion a physics switch put the
    /// document's materials through, so it stays pending in the new physics.
    /// Left as it was, it read as an edit of the converted material, and Apply
    /// wrote the old physics' values over it. One that will not convert goes,
    /// and the panel takes the document's; half-typed formula text, in the old
    /// physics' numbers, goes as an undo drops it.
    pub(super) fn convert(&mut self, from: PhysicsModel, to: PhysicsModel) {
        self.staged = self
            .staged
            .take()
            .and_then(|edit| from.convert_material(to, &edit).ok());
        self.formulas.clear();
        self.errors.clear();
    }

    /// Whether the open material has edits `draft` does not have yet.
    pub(super) fn pending(&self, draft: &TopologyScene) -> bool {
        self.staged
            .as_ref()
            .is_some_and(|edit| edit.id == self.selection && draft.material(edit.id) != Some(edit))
    }

    /// The open material, which a drawn subdomain is given too. The selection
    /// outlives the document it was made in - a scene load, an undo past the
    /// material's creation - so it is resolved against the draft each time it
    /// is used rather than trusted: the default material if the draft has it,
    /// otherwise its first.
    pub(super) fn resolved(&mut self, draft: &TopologyScene) -> MaterialId {
        let materials = &draft.materials;
        if !materials
            .iter()
            .any(|material| material.id == self.selection)
        {
            self.selection = materials
                .iter()
                .find(|material| material.id == DEFAULT_MATERIAL)
                .or_else(|| materials.first())
                .map_or(DEFAULT_MATERIAL, |material| material.id);
        }
        self.selection
    }

    /// Stages the open material from `draft` unless a copy of it is staged
    /// already.
    pub(super) fn stage(&mut self, draft: &TopologyScene) {
        if self
            .staged
            .as_ref()
            .is_none_or(|material| material.id != self.selection)
        {
            self.staged = draft.material(self.selection).cloned();
        }
    }
}
