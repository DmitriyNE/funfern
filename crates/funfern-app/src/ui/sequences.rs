//! Generated sequences of actions on the Materials panel, run through the
//! panel's own widgets.
//!
//! The defects this guards against are disagreements between components about
//! what is current: formula text against the material it was typed for, a
//! staged copy against the document, a selection against the material it
//! opened. Each component was right alone, and each defect took a particular
//! order of ordinary actions. A focused test documents one such order; these
//! tests draw orders of legal actions and, when one fails, shrink it to the
//! few steps that matter, which is a focused test waiting to be written.
//!
//! The reference state carries what the generator needs, a step counter so
//! that typed text is fresh each time. The oracle is beside the system under
//! test: a shadow history kept from the commits observed, which says what
//! Undo and Redo must bring back and how deep the history is; and a
//! post-condition on each action, which says what it may and may not have
//! touched. Those are claims about ownership and order, not about laws, so
//! nothing here recomputes a material.
//!
//! `PROPTEST_CASES` runs more sequences than the gate's few; a failure is
//! written under `proptest-regressions` and replayed first on the next run.
//! A seed names a sequence only through this generator: change an action, a
//! weight or a text, and every seed there replays something else. So each
//! finding gets a focused test of its own, and the seeds kept are only those
//! of findings drawn by the generator as it stands.
//!
//! `reopening` runs the same panel with the document's other edits among its
//! actions, saved and opened again between any two. `generations` runs the
//! runtime under interleavings of host frames, device completions and user
//! actions, against what it has watched the device come to run.

use super::test_support::*;
use super::*;
use bevy_egui::egui::accesskit::Role;
use funfern_app::topology_editor::{
    ClosedCurvePurpose, OpenCurvePurpose, TopologyAcceptance, TopologyAttachment, TopologyDocument,
    TopologyDocumentModel, TopologyProbeTarget,
};
use funfern_app::topology_persistence as persistence;
use funfern_app::topology_viewport::{TopologyHandle, plan_handle_drag};
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

mod generations;
mod reopening;

/// One action on the panel, as a user takes it. Ordinals count the widgets
/// of a kind top to bottom and wrap, so every value is legal in every state
/// and a shortened sequence stays legal.
#[derive(Clone, Debug)]
enum Action {
    /// Click the `field`th text field, select its text, type `text` and
    /// press Enter.
    Type {
        field: u8,
        text: String,
    },
    /// Click the `field`th text field and press Enter without typing.
    EnterUntouched {
        field: u8,
    },
    /// Open the `combo`th combo box and pick its `item`th entry.
    Pick {
        combo: u8,
        item: u8,
    },
    Press(Button),
    /// Open or close a collapsing header.
    Fold(Header),
    /// Toggle the open material's Advanced view.
    Advanced,
    /// Click the `nth` material in the Library.
    SelectMaterial(u8),
    /// Select the `nth` region, as a click on its face does.
    SelectRegion(u8),
    Undo,
    Redo,
    /// Switch the document to the `nth` physics model.
    Physics(u8),
    New,
    /// Let the draft's validation finish.
    Settle,
    /// Click the `nth` checkbox.
    Toggle(u8),
    /// Save the document and open it again by `Route`, as a reload does.
    Reopen(Route),
    /// Draw a rounded closed curve in the `cell`th of sixteen cells of the
    /// domain, a subdomain of the open material or a hole, `size` choosing
    /// its radius, as the Draw tool's finish does.
    Draw {
        hole: bool,
        cell: u8,
        size: u8,
    },
    /// Draw a separator of the Draw palette's material from the floor to the
    /// ceiling, at the `at`th of three places.
    Divide {
        at: u8,
    },
    /// Draw a free baffle through the `cell`th cell, turned `turn` sixteenths.
    Baffle {
        cell: u8,
        turn: u8,
    },
    /// Drag a control point of the `curve`th curve by the `step`th offset,
    /// as the viewport's handle drag does in one go.
    Nudge {
        curve: u8,
        control: u8,
        step: u8,
    },
    /// Delete the `curve`th curve, keeping the `keep`th region of those a
    /// merge would offer.
    Remove {
        curve: u8,
        keep: u8,
    },
    /// Let go of an end of the `curve`th open curve.
    Detach {
        curve: u8,
        end: u8,
    },
    /// Place a point probe in the `cell`th cell.
    Probe {
        cell: u8,
    },
    /// Move the point source to the `cell`th cell, on or off.
    Source {
        cell: u8,
        on: bool,
    },
    /// Resize the domain about its centre, the `nth` of four ways.
    Domain(u8),
}

/// How a saved document comes back.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// Save scene's file, opened by Open: the pretty form, through the file
    /// event the loader handles. The native autosave writes the same form.
    File,
    /// A shared link, opened at launch: the compact form, compressed and
    /// encoded into the address.
    Link,
    /// The browser's autosave, restored at launch: the compact form.
    Autosave,
}

#[derive(Clone, Copy, Debug)]
enum Button {
    Apply,
    Revert,
    AddMaterial,
    DeleteMaterial,
    AddParameter,
    /// The `−` of the nth parameter row.
    DeleteParameter(u8),
}

#[derive(Clone, Copy, Debug)]
enum Header {
    Parameters,
    Anisotropy,
    Restoring,
    ShortWave,
}

impl Header {
    fn label(self) -> &'static str {
        match self {
            Header::Parameters => "Parameters",
            Header::Anisotropy => "Anisotropy",
            Header::Restoring => "Restoring force",
            Header::ShortWave => "Short-wave loss",
        }
    }
}

/// Texts to type at `step`: a constant above every field's minimum, a formula
/// in the coordinates, one that names a parameter a material may or may not
/// have, an identifier that is a name or an unknown, and one that does not
/// parse. The step keeps each fresh, so typing is never a no-op by accident.
fn texts(step: u32) -> impl Strategy<Value = String> {
    prop_oneof![
        Just(format!("{}.5", 2 + step % 9)),
        Just(format!("1 + {}*x*x", 1 + step % 5)),
        Just("p1*2".to_owned()),
        Just(format!("q{step}")),
        Just("(".to_owned()),
    ]
}

/// The reference state: how many actions have been taken.
#[derive(Clone, Debug)]
struct Steps {
    taken: u32,
}

struct MaterialPanel;

impl ReferenceStateMachine for MaterialPanel {
    type State = Steps;
    type Transition = Action;

    fn init_state() -> BoxedStrategy<Steps> {
        Just(Steps { taken: 0 }).boxed()
    }

    fn transitions(state: &Steps) -> BoxedStrategy<Action> {
        panel_actions(state.taken)
    }

    fn apply(state: Steps, _: &Action) -> Steps {
        Steps {
            taken: state.taken + 1,
        }
    }
}

/// The Materials panel's own actions at `step`. The seeds kept for this
/// machine name sequences through exactly this strategy: changing an arm or
/// a weight here makes them replay something else.
fn panel_actions(step: u32) -> BoxedStrategy<Action> {
    prop_oneof![
        25 => (any::<u8>(), texts(step)).prop_map(|(field, text)| Action::Type { field, text }),
        10 => any::<u8>().prop_map(|field| Action::EnterUntouched { field }),
        15 => (any::<u8>(), any::<u8>()).prop_map(|(combo, item)| Action::Pick { combo, item }),
        8 => Just(Action::Press(Button::Apply)),
        4 => Just(Action::Press(Button::Revert)),
        3 => Just(Action::Press(Button::AddMaterial)),
        1 => Just(Action::Press(Button::DeleteMaterial)),
        4 => Just(Action::Press(Button::AddParameter)),
        3 => any::<u8>().prop_map(|nth| Action::Press(Button::DeleteParameter(nth))),
        // Parameters more often than the rest: its rows are where names
        // are typed and parameters added or deleted.
        6 => prop_oneof![
            3 => Just(Header::Parameters),
            1 => Just(Header::Anisotropy),
            1 => Just(Header::Restoring),
            1 => Just(Header::ShortWave),
        ]
        .prop_map(Action::Fold),
        5 => Just(Action::Advanced),
        5 => any::<u8>().prop_map(Action::SelectMaterial),
        5 => any::<u8>().prop_map(Action::SelectRegion),
        5 => Just(Action::Undo),
        3 => Just(Action::Redo),
        2 => any::<u8>().prop_map(Action::Physics),
        1 => Just(Action::New),
        4 => Just(Action::Settle),
    ]
    .boxed()
}

/// The panel under test: the playground, the context that shows its
/// Materials panel, what the last pass laid out, and the shadow history.
struct Panel {
    state: Playground,
    driver: PanelDriver,
    widgets: Vec<LaidOut>,
    /// The document models each Undo must bring back, newest last, kept from
    /// the commits observed.
    undo: Vec<TopologyDocumentModel>,
    /// The ones Redo must, since the last Undo.
    redo: Vec<TopologyDocumentModel>,
}

/// The authored part of a document model: what the history records. The
/// accepted scene follows the draft when validation finishes, which is not
/// an edit and happens at no particular action.
fn authored(model: &TopologyDocumentModel) -> TopologyDocumentModel {
    TopologyDocumentModel {
        accepted: model.draft.clone(),
        ..model.clone()
    }
}

/// The centre of the `cell`th of sixteen cells of `domain`, row by row.
fn cell_point(domain: DomainRect, cell: u8) -> Point2 {
    let (column, row) = (f64::from(cell % 4), f64::from(cell / 4 % 4));
    Point2::new(
        domain.min_x + (column + 0.5) / 4.0 * domain.width(),
        domain.min_y + (row + 0.5) / 4.0 * domain.height(),
    )
}

/// Every id a model holds, by kind, in the draft or in the accepted scene:
/// what the editor's six allocators must never hand out again.
fn ids(model: &TopologyDocumentModel, accepted: bool) -> BTreeSet<(&'static str, u64)> {
    let scene = if accepted {
        &model.accepted
    } else {
        &model.draft
    };
    let curves = &scene.geometry.curves;
    curves
        .iter()
        .map(|curve| ("curve", curve.id.0))
        .chain(
            curves
                .iter()
                .flat_map(|curve| curve.spans.iter().map(|span| ("span", span.id.0))),
        )
        .chain(
            scene
                .geometry
                .vertices
                .iter()
                .map(|vertex| ("vertex", vertex.id.0)),
        )
        .chain(scene.regions.iter().map(|region| ("region", region.id.0)))
        .chain(
            scene
                .materials
                .iter()
                .map(|material| ("material", material.id.0)),
        )
        .chain(model.probes.iter().map(|probe| ("probe", probe.id.0)))
        .collect()
}

/// What a file keeps of `document`. The session holds a deleted material's
/// Advanced view and a deleted probe's readout, so that an Undo brings them
/// back as they were; a file has no Undo and keeps neither
/// (`shown_materials` and the probe readouts in `encode_document`).
fn kept(document: &TopologyDocument) -> TopologyDocument {
    let mut kept = document.clone();
    let TopologyDocument {
        model,
        presentation,
        readouts,
    } = &mut kept;
    presentation
        .advanced_materials
        .retain(|id| model.draft.material(id).is_some());
    readouts
        .probes
        .retain(|id, _| model.probes.iter().any(|probe| probe.id == *id));
    kept
}

/// What an action may or may not have changed, read before and after it.
#[derive(Clone, Debug, PartialEq)]
struct Observed {
    model: TopologyDocumentModel,
    staged: Option<Material>,
    selection: MaterialId,
}

impl Panel {
    fn pass(&mut self, events: Vec<egui::Event>) {
        let Panel {
            state,
            driver,
            widgets,
            ..
        } = self;
        *widgets = driver.pass(events, |ui| state.materials_panel(ui));
    }

    fn observe(&self) -> Observed {
        Observed {
            model: authored(&self.state.editor.document.model),
            staged: self.state.material_session.staged.clone(),
            selection: self.state.material_session.selection,
        }
    }

    fn of_role(&self, role: Role) -> Vec<LaidOut> {
        self.widgets
            .iter()
            .filter(|widget| widget.role == role)
            .cloned()
            .collect()
    }

    fn nth(items: &[LaidOut], nth: u8) -> Option<LaidOut> {
        (!items.is_empty()).then(|| items[nth as usize % items.len()].clone())
    }

    fn button(&self, label: &str) -> Option<LaidOut> {
        self.widgets
            .iter()
            .find(|widget| widget.role == Role::Button && widget.label == label)
            .cloned()
    }

    /// Whether the action said something in the status line.
    fn noticed(&self) -> bool {
        !self.state.message.is_empty()
    }

    fn committed(&self, material: MaterialId) -> Option<Material> {
        self.state
            .editor
            .document
            .model
            .draft
            .material(material)
            .cloned()
    }

    fn materials(&self) -> Vec<Material> {
        self.state.editor.document.model.draft.materials.clone()
    }

    fn act(&mut self, action: &Action) {
        match action {
            Action::Type { field, text } => {
                let fields = self.of_role(Role::TextInput);
                let Some(target) = Self::nth(&fields, *field) else {
                    return;
                };
                self.pass(click(&target));
                self.pass(typed(text));
                self.pass(vec![]);
                // What was typed is on screen, or the panel refused it and
                // said so. Nothing typed is dropped or replaced quietly. Found
                // by its text rather than its place: a law edited by hand
                // leaves its preset, and the fields are laid out anew.
                let fields = self.of_role(Role::TextInput);
                assert!(
                    fields.iter().any(|shown| shown.label == *text) || self.noticed(),
                    "typed {text:?} into {target:?}: no field shows it and nothing was said"
                );
            }
            Action::EnterUntouched { field } => {
                let fields = self.of_role(Role::TextInput);
                let Some(target) = Self::nth(&fields, *field) else {
                    return;
                };
                let before = self.observe();
                self.pass(click(&target));
                self.pass(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
                self.pass(vec![]);
                // A field nobody typed into commits nothing: what it shows
                // is what the material has.
                assert_eq!(
                    self.observe(),
                    before,
                    "Enter in the untouched field {target:?} changed something"
                );
                let fields = self.of_role(Role::TextInput);
                assert_eq!(
                    Self::nth(&fields, *field).map(|shown| shown.label),
                    Some(target.label.clone()),
                    "Enter in an untouched field changed its text"
                );
            }
            Action::Pick { combo, item } => {
                let combos = self.of_role(Role::ComboBox);
                let Some(target) = Self::nth(&combos, *combo) else {
                    return;
                };
                let before = self
                    .widgets
                    .iter()
                    .map(|widget| (widget.label.clone(), widget.rect.min.to_vec2().to_pos2()))
                    .collect::<Vec<_>>();
                self.pass(click(&target));
                // A new area is laid out disabled on its first frame, to be
                // measured; its entries take a click from the next one.
                self.pass(vec![]);
                // The popup's entries are the buttons that were not there.
                let entries = self
                    .widgets
                    .iter()
                    .filter(|widget| {
                        widget.role == Role::Button
                            && !before.contains(&(widget.label.clone(), widget.rect.min))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                match Self::nth(&entries, *item) {
                    Some(entry) => self.pass(click(&entry)),
                    None => self.pass(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
                }
                self.pass(vec![]);
            }
            Action::Press(Button::Apply) => {
                let Some(apply) = self.button("Apply") else {
                    return;
                };
                let before = self.observe();
                let others = |materials: Vec<Material>| {
                    materials
                        .into_iter()
                        .filter(|material| material.id != before.selection)
                        .collect::<Vec<_>>()
                };
                let others_before = others(self.materials());
                self.pass(click(&apply));
                self.pass(vec![]);
                if !apply.enabled || self.noticed() {
                    // Nothing to apply, or a refusal: the document stands.
                    assert_eq!(self.observe().model, before.model);
                    return;
                }
                // Apply commits the staged copy, exactly, and only it.
                assert_eq!(
                    self.committed(before.selection),
                    before.staged,
                    "Apply committed something other than the staged copy"
                );
                assert_eq!(others(self.materials()), others_before);
                assert!(!self.state.material_edits_pending());
            }
            Action::Press(Button::Revert) => {
                let Some(revert) = self.button("Revert") else {
                    return;
                };
                let before = self.observe();
                self.pass(click(&revert));
                self.pass(vec![]);
                assert_eq!(
                    self.observe().model,
                    before.model,
                    "Revert edited the document"
                );
                assert_eq!(
                    self.state.material_session.staged,
                    self.committed(before.selection),
                    "Revert left the staged copy different from the document's"
                );
            }
            Action::Press(Button::AddMaterial) => {
                let Some(add) = self.button("+") else {
                    return;
                };
                let count = self.materials().len();
                self.pass(click(&add));
                self.pass(vec![]);
                if count >= MAX_MATERIALS {
                    assert!(self.noticed(), "a full library refused quietly");
                    return;
                }
                let materials = self.materials();
                assert_eq!(materials.len(), count + 1);
                let added = materials.last().unwrap().id;
                assert_eq!(
                    self.state.material_session.selection, added,
                    "the new material opens"
                );
                assert_eq!(self.state.material_session.staged, self.committed(added));
            }
            Action::Press(Button::DeleteMaterial) => {
                let Some(delete) = self.button("Delete material") else {
                    return;
                };
                let doomed = self.state.material_session.selection;
                self.pass(click(&delete));
                self.pass(vec![]);
                assert!(self.committed(doomed).is_none(), "the material stayed");
                assert_eq!(self.state.material_session.selection, DEFAULT_MATERIAL);
                assert_eq!(
                    self.state.material_session.staged,
                    self.committed(DEFAULT_MATERIAL)
                );
            }
            Action::Press(Button::AddParameter) => {
                let Some(add) = self.button("+ Parameter") else {
                    return;
                };
                let before = self.observe();
                let count = before.staged.as_ref().unwrap().parameters.len();
                self.pass(click(&add));
                self.pass(vec![]);
                assert_eq!(
                    self.observe().model,
                    before.model,
                    "a parameter is staged, not committed"
                );
                assert_eq!(
                    self.state
                        .material_session
                        .staged
                        .as_ref()
                        .unwrap()
                        .parameters
                        .len(),
                    count + 1
                );
            }
            Action::Press(Button::DeleteParameter(nth)) => {
                let minuses = self
                    .widgets
                    .iter()
                    .filter(|widget| widget.role == Role::Button && widget.label == "−")
                    .cloned()
                    .collect::<Vec<_>>();
                let Some(minus) = Self::nth(&minuses, *nth) else {
                    return;
                };
                let before = self.observe();
                let count = before.staged.as_ref().unwrap().parameters.len();
                self.pass(click(&minus));
                self.pass(vec![]);
                assert_eq!(
                    self.observe().model,
                    before.model,
                    "a deletion is staged, not committed"
                );
                let after = self
                    .state
                    .material_session
                    .staged
                    .as_ref()
                    .unwrap()
                    .parameters
                    .len();
                if minus.enabled {
                    assert_eq!(after, count - 1, "the parameter stayed");
                } else {
                    assert_eq!(after, count, "a parameter in use was deleted");
                }
            }
            Action::Fold(header) => {
                let Some(fold) = self.button(header.label()) else {
                    return;
                };
                let before = self.observe();
                self.pass(click(&fold));
                self.pass(vec![]);
                assert_eq!(
                    self.observe(),
                    before,
                    "folding {header:?} edited something"
                );
            }
            Action::Advanced => {
                let Some(toggle) = self
                    .widgets
                    .iter()
                    .find(|widget| widget.role == Role::CheckBox && widget.label == "Advanced view")
                    .cloned()
                else {
                    return;
                };
                let before = self.observe();
                let advanced = self.state.material_advanced(before.selection);
                self.pass(click(&toggle));
                self.pass(vec![]);
                assert_eq!(self.state.material_advanced(before.selection), !advanced);
                // A view, not an edit: pending edits stay pending, as they are.
                assert_eq!(self.observe(), before, "the Advanced view edited something");
            }
            Action::SelectMaterial(nth) => {
                let materials = self.materials();
                let index = *nth as usize % materials.len();
                let target = materials[index].clone();
                // The Library's rows, in the materials' order: the buttons
                // below its heading named as a material is. A region row of
                // the roster above may carry the same name, and so may
                // another material.
                let Some(heading) = self
                    .widgets
                    .iter()
                    .find(|widget| widget.role == Role::Label && widget.label == "Library")
                else {
                    return;
                };
                let rows = self
                    .widgets
                    .iter()
                    .filter(|widget| {
                        widget.role == Role::Button
                            && widget.rect.min.y > heading.rect.min.y
                            && materials
                                .iter()
                                .any(|material| material.name == widget.label)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let Some(entry) = rows.get(index).cloned() else {
                    return;
                };
                let before = self.observe();
                self.pass(click(&entry));
                self.pass(vec![]);
                assert_eq!(
                    self.observe().model,
                    before.model,
                    "selecting edited the document"
                );
                assert_eq!(self.state.material_session.selection, target.id);
                assert_eq!(
                    self.state.material_session.staged,
                    self.committed(target.id)
                );
            }
            Action::SelectRegion(nth) => {
                let regions = self.state.editor.document.model.draft.regions.clone();
                let target = regions[*nth as usize % regions.len()];
                let before = self.observe();
                let pending = self.state.material_edits_pending();
                self.state.select_region(target.id);
                self.pass(vec![]);
                assert_eq!(
                    self.observe().model,
                    before.model,
                    "selecting edited the document"
                );
                if pending && target.material != before.selection {
                    // Following the selection would drop the edits: refused,
                    // and said.
                    assert!(
                        self.noticed(),
                        "a selection away from pending edits was not refused"
                    );
                    assert_eq!(self.state.material_session.selection, before.selection);
                    assert_eq!(self.state.material_session.staged, before.staged);
                } else {
                    assert_eq!(self.state.region_selection, target.id);
                    assert_eq!(self.state.material_session.selection, target.material);
                    // Following to another material opens it as the document
                    // has it; staying on the open one keeps what was staged.
                    let expected = if target.material == before.selection {
                        before.staged
                    } else {
                        self.committed(target.material)
                    };
                    assert_eq!(self.state.material_session.staged, expected);
                }
            }
            Action::Undo => {
                let moves = !self.undo.is_empty();
                self.state.undo();
                self.pass(vec![]);
                // A step back drops what was staged against the step left.
                if moves {
                    assert!(
                        !self.state.material_edits_pending(),
                        "Undo kept pending edits"
                    );
                }
            }
            Action::Redo => {
                let moves = !self.redo.is_empty();
                self.state.redo();
                self.pass(vec![]);
                if moves {
                    assert!(
                        !self.state.material_edits_pending(),
                        "Redo kept pending edits"
                    );
                }
            }
            Action::Physics(nth) => {
                let models = [
                    PhysicsModel::Mechanical,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm,
                    },
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Te,
                    },
                ];
                let target = models[*nth as usize % models.len()];
                let before = self.observe();
                let pending = self.state.material_edits_pending();
                let previous = self.state.editor.document.model.draft.physics;
                // As the Physics combo does.
                match self.state.editor.set_physics(target) {
                    Ok(()) => self.state.convert_material_edits(previous, target),
                    Err(error) => self.state.message = error,
                }
                self.pass(vec![]);
                if self.noticed() || target == previous {
                    assert_eq!(self.observe().model, before.model);
                    return;
                }
                assert_eq!(self.state.editor.document.model.draft.physics, target);
                for material in before.model.draft.materials {
                    assert_eq!(
                        self.committed(material.id),
                        previous.convert_material(target, &material).ok(),
                        "a material was not converted with the physics"
                    );
                }
                // An edit pending through the switch stays pending, converted;
                // one that will not convert goes, and the panel takes the
                // document's.
                let expected = pending
                    .then(|| {
                        previous
                            .convert_material(target, before.staged.as_ref().unwrap())
                            .ok()
                    })
                    .flatten()
                    .or_else(|| self.committed(before.selection));
                assert_eq!(self.state.material_session.staged, expected);
            }
            Action::New => {
                self.state.new_scene();
                self.pass(vec![]);
                assert_eq!(
                    self.materials(),
                    TopologyDocument::default().model.draft.materials
                );
                assert_eq!(self.state.material_session.selection, DEFAULT_MATERIAL);
                assert_eq!(
                    self.state.material_session.staged,
                    self.committed(DEFAULT_MATERIAL)
                );
            }
            Action::Settle => {
                let before = self.observe();
                settle(&mut self.state.editor);
                self.pass(vec![]);
                assert_eq!(self.observe(), before, "validation edited something");
            }
            Action::Toggle(nth) => {
                let boxes = self.of_role(Role::CheckBox);
                let Some(target) = Self::nth(&boxes, *nth) else {
                    return;
                };
                self.pass(click(&target));
                self.pass(vec![]);
            }
            Action::Reopen(route) => self.reopen(*route),
            _ => {
                let whole = self.state.editor.document.model.clone();
                let result = self.edit_geometry(action);
                self.pass(vec![]);
                // A refused command leaves everything as it was.
                if let Err(error) = result {
                    assert_eq!(
                        self.state.editor.document.model, whole,
                        "{action:?} was refused ({error}) and changed the document"
                    );
                }
            }
        }
    }

    /// The document's other edits, each through the call its tool makes.
    fn edit_geometry(&mut self, action: &Action) -> Result<(), String> {
        let state = &mut self.state;
        let draft = &state.editor.document.model.draft;
        let domain = draft.geometry.domain;
        let span = domain.width().min(domain.height());
        let curves = draft.geometry.curves.clone();
        let pick = |nth: u8| (!curves.is_empty()).then(|| &curves[nth as usize % curves.len()]);
        match *action {
            Action::Draw { hole, cell, size } => {
                state.closed_purpose = if hole {
                    ClosedPurpose::Hole
                } else {
                    ClosedPurpose::Subdomain
                };
                let radius = [0.04, 0.08, 0.16][size as usize % 3] * span;
                state
                    .create_closed(PeriodicCubicSpline::rounded(
                        cell_point(domain, cell),
                        radius,
                    ))
                    .map(drop)
            }
            Action::Divide { at } => {
                let fraction = [0.25, 0.5, 0.75][at as usize % 3];
                let x = domain.min_x + fraction * domain.width();
                let y = |share: f64| domain.min_y + share * domain.height();
                let wall = |side, fraction| {
                    Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                        side,
                        fraction,
                    }))
                };
                let spline = OpenCubicSpline::polyline(vec![
                    Point2::new(x, y(0.1)),
                    Point2::new(x, y(0.5)),
                    Point2::new(x, y(0.9)),
                ])
                .map_err(|error| error.to_string())?;
                // The Top side runs right to left.
                state
                    .editor
                    .create_open_curve(
                        spline,
                        OpenCurvePurpose::SubdomainSeparator {
                            material: state.new_separator_material,
                        },
                        wall(OuterSide::Bottom, fraction),
                        wall(OuterSide::Top, 1.0 - fraction),
                    )
                    .map(drop)
            }
            Action::Baffle { cell, turn } => {
                let centre = cell_point(domain, cell);
                let angle = f64::from(turn % 16) * std::f64::consts::PI / 16.0;
                let reach = 0.12 * span;
                let (dx, dy) = (reach * angle.cos(), reach * angle.sin());
                let spline = OpenCubicSpline::polyline(vec![
                    Point2::new(centre.x - dx, centre.y - dy),
                    centre,
                    Point2::new(centre.x + dx, centre.y + dy),
                ])
                .map_err(|error| error.to_string())?;
                state.editor.create_boundary_baffle(spline).map(drop)
            }
            Action::Nudge {
                curve,
                control,
                step,
            } => {
                let Some(target) = pick(curve) else {
                    return Ok(());
                };
                let controls = match &target.spline {
                    CurveSpline::Closed(spline) => spline.controls(),
                    CurveSpline::Open(spline) => spline.controls(),
                };
                let index = control as usize % controls.len();
                let (dx, dy) = [
                    (0.02, 0.0),
                    (0.0, -0.03),
                    (0.12, 0.08),
                    (-0.35, 0.25),
                    (0.8, -0.1),
                ][step as usize % 5];
                let point =
                    Point2::new(controls[index].x + dx * span, controls[index].y + dy * span);
                let update = plan_handle_drag(
                    &draft.geometry,
                    TopologyHandle::Control {
                        curve: target.id,
                        control: index,
                    },
                    point,
                )
                .map_err(|error| error.to_string())?;
                state.editor.apply_transform_updates(&[update])
            }
            Action::Remove { curve, keep } => {
                let Some(target) = pick(curve) else {
                    return Ok(());
                };
                let choices = state.editor.curve_removal_choices(target.id)?;
                let kept = (!choices.is_empty()).then(|| choices[keep as usize % choices.len()]);
                state.editor.remove_curve(target.id, kept).map(drop)
            }
            Action::Detach { curve, end } => {
                let open = curves
                    .iter()
                    .filter(|curve| matches!(curve.spline, CurveSpline::Open(_)))
                    .collect::<Vec<_>>();
                let Some(target) = (!open.is_empty()).then(|| open[curve as usize % open.len()])
                else {
                    return Ok(());
                };
                state.editor.detach_endpoint(target.id, end as usize % 2)
            }
            Action::Probe { cell } => {
                let name = format!("Probe {}", state.editor.document.model.probes.len() + 1);
                state
                    .editor
                    .create_probe(
                        name,
                        [91, 220, 194],
                        TopologyProbeTarget::Point(cell_point(domain, cell)),
                    )
                    .map(drop)
            }
            Action::Source { cell, on } => {
                let mut source = state.editor.document.model.source;
                source.position = cell_point(domain, cell);
                source.enabled = on;
                state.editor.set_point_source(source)
            }
            Action::Domain(nth) => {
                let (centre_x, centre_y) = (
                    0.5 * (domain.min_x + domain.max_x),
                    0.5 * (domain.min_y + domain.max_y),
                );
                let [wide, tall] =
                    [[1.25, 1.25], [0.8, 0.8], [1.3, 1.0], [1.0, 0.7]][nth as usize % 4];
                state.editor.set_domain(DomainRect {
                    min_x: centre_x - 0.5 * wide * domain.width(),
                    max_x: centre_x + 0.5 * wide * domain.width(),
                    min_y: centre_y - 0.5 * tall * domain.height(),
                    max_y: centre_y + 0.5 * tall * domain.height(),
                })
            }
            _ => unreachable!("not a geometry edit: {action:?}"),
        }
    }

    /// Saves the document by `route` and opens it again through that route's
    /// own handler, as a reload does. The document comes back as it was,
    /// invalid draft and accepted scene alike, and validation brings it where
    /// the session would have come.
    fn reopen(&mut self, route: Route) {
        let saved = self.state.editor.document.clone();
        let bytes = match route {
            Route::File => persistence::save(&saved).map(String::into_bytes),
            Route::Link => crate::sharing::encode(&saved)
                .and_then(|fragment| crate::sharing::decode(&fragment)),
            Route::Autosave => persistence::save_compact(&saved),
        }
        .unwrap_or_else(|error| panic!("the document would not save by {route:?}: {error}"));
        settle(&mut self.state.editor);
        let settled = (
            self.state.editor.acceptance,
            self.state.editor.document.clone(),
        );
        match route {
            Route::File => {
                self.state
                    .sender
                    .send(FileEvent::Loaded(bytes))
                    .expect("the file channel is open");
                self.state.update_files();
            }
            Route::Link => self
                .state
                .open_startup_scene(Some(Ok(bytes)), || Ok(None), true),
            Route::Autosave => self
                .state
                .open_startup_scene(None, || Ok(Some(bytes)), true),
        }
        let notices = self
            .state
            .notices
            .iter()
            .map(|notice| format!("{}: {}", notice.title, notice.text))
            .collect::<Vec<_>>();
        assert!(
            notices.is_empty(),
            "reopening by {route:?} said {notices:?}"
        );
        assert_eq!(
            self.state.editor.document,
            kept(&saved),
            "reopening by {route:?} changed the document"
        );
        settle(&mut self.state.editor);
        assert_eq!(
            (self.state.editor.acceptance, &self.state.editor.document),
            (settled.0, &kept(&settled.1)),
            "reopened by {route:?}, validation came somewhere the session did not"
        );
        self.pass(vec![]);
    }

    /// The shadow history's step for `action`, from what it was observed to do.
    fn record(&mut self, action: &Action, whole: TopologyDocumentModel) {
        let before = authored(&whole);
        let after = authored(&self.state.editor.document.model);
        match action {
            Action::Undo => match self.undo.pop() {
                Some(expected) => {
                    assert_eq!(
                        after,
                        authored(&expected),
                        "Undo brought back something else"
                    );
                    self.redo.push(before);
                }
                None => assert_eq!(
                    after, before,
                    "Undo with nothing to undo changed the document"
                ),
            },
            Action::Redo => match self.redo.pop() {
                Some(expected) => {
                    assert_eq!(
                        after,
                        authored(&expected),
                        "Redo brought back something else"
                    );
                    self.undo.push(before);
                }
                None => assert_eq!(
                    after, before,
                    "Redo with nothing to redo changed the document"
                ),
            },
            // An opened file, link or autosave starts the history afresh.
            Action::Reopen(_) => {
                self.undo.clear();
                self.redo.clear();
            }
            // A replacement is a step unless it brings the same document,
            // accepted scene and all, which is what an edit's commit weighs
            // too; validation moves the accepted scene, not an action.
            Action::New => {
                if self.state.editor.document.model != whole {
                    self.undo.push(before);
                    self.redo.clear();
                }
            }
            _ => {
                if after != before {
                    self.undo.push(before);
                    self.redo.clear();
                }
            }
        }
    }
}

impl Panel {
    /// The default scene with a second material and a subdomain made of it,
    /// so a selection can move between materials. The two edits that build
    /// it are the history's first steps.
    fn start() -> Self {
        let mut state = Playground::default();
        let mut undo = vec![state.editor.document.model.clone()];
        let second = state.editor.add_material().unwrap();
        undo.push(state.editor.document.model.clone());
        state
            .editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.5, 0.3), 0.15),
                ClosedCurvePurpose::Subdomain { material: second },
            )
            .unwrap();
        settle(&mut state.editor);
        let mut panel = Panel {
            state,
            driver: PanelDriver::new(),
            widgets: vec![],
            undo,
            redo: vec![],
        };
        panel.pass(vec![]);
        panel
    }

    fn step(mut self, action: Action) -> Self {
        let before = self.state.editor.document.model.clone();
        self.state.message.clear();
        self.state.notices.clear();
        self.act(&action);
        // An id handed out is one nothing has held: not the accepted scene,
        // and not any step the history can bring back. Undo, Redo and a scene
        // replacement bring ids back rather than hand them out.
        if !matches!(
            action,
            Action::Undo | Action::Redo | Action::New | Action::Reopen(_)
        ) {
            let held = ids(&before, true)
                .into_iter()
                .chain(
                    self.undo
                        .iter()
                        .chain(&self.redo)
                        .flat_map(|model| ids(model, false)),
                )
                .collect::<BTreeSet<_>>();
            let fresh = ids(&self.state.editor.document.model, false)
                .difference(&ids(&before, false))
                .copied()
                .collect::<BTreeSet<_>>();
            let reused = fresh.intersection(&held).collect::<Vec<_>>();
            assert!(
                reused.is_empty(),
                "{action:?} handed out held ids {reused:?}"
            );
        }
        self.record(&action, before);
        self
    }

    fn check(&self) {
        let state = &self.state;
        assert!(!state.editor.editing(), "an edit was left open");
        let model = &state.editor.document.model;
        let draft = &model.draft;
        for material in &draft.materials {
            assert!(
                material.valid(),
                "{:?} is in the document invalid",
                material.name
            );
        }
        // Valid says the accepted scene is the draft, validated.
        if state.editor.acceptance == TopologyAcceptance::Valid {
            assert_eq!(
                model.accepted, model.draft,
                "Valid, with an accepted scene that is not the draft"
            );
        }
        // The autosave writes the document within a second of any change.
        if let Err(error) = persistence::save_compact(&state.editor.document) {
            panic!("the document would not save: {error}");
        }
        assert_eq!(
            state.editor.history_len(),
            (self.undo.len(), self.redo.len()),
            "the history is not as deep as the commits observed"
        );
        // The open material is in the draft, and the staged copy is of it.
        let open = state.material_session.selection;
        assert!(draft.material(open).is_some(), "the open material is gone");
        let staged = state
            .material_session
            .staged
            .as_ref()
            .expect("a pass leaves a staged copy");
        assert_eq!(staged.id, open, "the staged copy is of another material");
        // Apply is offered exactly while the staged copy differs.
        let apply = self
            .widgets
            .iter()
            .find(|widget| widget.role == Role::Button && widget.label == "Apply")
            .expect("Apply is always shown");
        assert_eq!(
            apply.enabled,
            state.material_edits_pending(),
            "Apply is offered against the staged copy's state"
        );
    }
}

impl StateMachineTest for MaterialPanel {
    type SystemUnderTest = Panel;
    type Reference = Self;

    fn init_test(_: &Steps) -> Panel {
        Panel::start()
    }

    fn apply(panel: Panel, _: &Steps, action: Action) -> Panel {
        panel.step(action)
    }

    fn check_invariants(panel: &Panel, _: &Steps) {
        panel.check();
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]
    #[test]
    fn material_edits_in_any_order_leave_the_panel_and_the_document_agreed(
        sequential 1..40 => MaterialPanel
    );
}
