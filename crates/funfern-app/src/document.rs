//! Shared persisted presentation and measurement vocabulary.
//!
//! These types are independent of both the retired object-specific editor and
//! the topology editor. Production topology code imports them from here.

use funfern_core::{ElectromagneticPolarization, MAX_MATERIALS, MaterialId, PhysicsModel};

pub const MAX_PROBES: usize = 16;
pub const MAX_SEGMENT_PROBE_POINTS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialOverlay {
    Off,
    /// Every face takes the colour of the material assigned to it.
    Regions,
    /// Every face takes a categorical colour keyed by its stable `RegionId`, so
    /// neighbouring subdomains that happen to share a material stay distinct.
    Subdomains,
    /// Every element takes the edge length the adaptation estimate last asked
    /// for. It shares the slot with the material overlays because it fills the
    /// domain the same way: under a field that turns translucent over it.
    AdaptationTarget,
    Property(MaterialProperty),
}

impl MaterialOverlay {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Regions => "Materials",
            Self::Subdomains => "Subdomains",
            Self::AdaptationTarget => "Adaptation target",
            Self::Property(property) => property.label(),
        }
    }

    pub const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Regions => "Materials",
            Self::Subdomains => "Subdomains",
            Self::AdaptationTarget => "Adaptation target",
            Self::Property(property) => property.label_for(physics),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialProperty {
    Density,
    Stiffness,
    Damping,
    WaveSpeed,
    Impedance,
    Anisotropy,
    VolumeSource,
}

impl MaterialProperty {
    pub const ALL: [Self; 7] = [
        Self::Density,
        Self::Stiffness,
        Self::Damping,
        Self::WaveSpeed,
        Self::Impedance,
        Self::Anisotropy,
        Self::VolumeSource,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Density => "Density",
            Self::Stiffness => "Stiffness",
            Self::Damping => "Damping",
            Self::WaveSpeed => "Wave speed",
            Self::Impedance => "Impedance",
            Self::Anisotropy => "Material anisotropy",
            Self::VolumeSource => "Volume source",
        }
    }

    pub const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match physics {
            PhysicsModel::Mechanical => self.label(),
            PhysicsModel::Electromagnetic { .. } => match self {
                Self::Density => "Permittivity ε",
                Self::Stiffness => "Permeability μ",
                Self::Damping => "Loss rate α",
                Self::WaveSpeed => "Wave speed",
                Self::Impedance => "Wave impedance",
                Self::Anisotropy => "Material anisotropy",
                Self::VolumeSource => "Volume current",
            },
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Density => 0,
            Self::Stiffness => 1,
            Self::Damping => 2,
            Self::WaveSpeed => 3,
            Self::Impedance => 4,
            Self::Anisotropy => 5,
            Self::VolumeSource => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorOverlay {
    #[default]
    Off,
    ComplementaryField,
    RelativeEnergyFlow,
}

impl VectorOverlay {
    /// Mechanical uses the same canonical vector state internally, but that
    /// implementation detail is not one of its presentation observables.
    pub const fn choices(physics: PhysicsModel) -> &'static [Self] {
        match physics {
            PhysicsModel::Mechanical => &[Self::Off, Self::RelativeEnergyFlow],
            PhysicsModel::Electromagnetic { .. } => &[
                Self::Off,
                Self::ComplementaryField,
                Self::RelativeEnergyFlow,
            ],
        }
    }

    pub const fn resolved(self, physics: PhysicsModel) -> Self {
        match (self, physics) {
            (Self::ComplementaryField, PhysicsModel::Mechanical) => Self::Off,
            _ => self,
        }
    }

    pub const fn label(self, physics: PhysicsModel) -> &'static str {
        match (self.resolved(physics), physics) {
            (Self::Off, _) => "Off",
            (
                Self::ComplementaryField,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
            ) => "Magnetic field H",
            (
                Self::ComplementaryField,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
            ) => "Electric field E",
            // `resolved` prevents this combination; retain an exhaustive arm
            // so adding another skin cannot accidentally expose a core detail.
            (Self::ComplementaryField, PhysicsModel::Mechanical) => "Off",
            (Self::RelativeEnergyFlow, PhysicsModel::Electromagnetic { .. }) => "Poynting flow",
            (Self::RelativeEnergyFlow, PhysicsModel::Mechanical) => "Energy flow",
        }
    }
}

/// A set of material ids, at most one per material a scene can hold, so it
/// stays `Copy` with the rest of the view settings. Ids are kept sorted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdvancedMaterials {
    ids: [u64; MAX_MATERIALS],
    len: usize,
}

impl AdvancedMaterials {
    pub fn contains(&self, id: MaterialId) -> bool {
        self.ids[..self.len].binary_search(&id.0).is_ok()
    }

    pub fn iter(&self) -> impl Iterator<Item = MaterialId> + '_ {
        self.ids[..self.len].iter().map(|id| MaterialId(*id))
    }

    /// Marks `id` as shown in Advanced or not. A set holding ids of materials
    /// that no longer exist can be full; [`Self::retain`] against the scene
    /// first, which leaves room for every material it can hold.
    pub fn set(&mut self, id: MaterialId, advanced: bool) {
        match (self.ids[..self.len].binary_search(&id.0), advanced) {
            (Err(index), true) if self.len < MAX_MATERIALS => {
                self.ids.copy_within(index..self.len, index + 1);
                self.ids[index] = id.0;
                self.len += 1;
            }
            (Ok(index), false) => {
                self.ids.copy_within(index + 1..self.len, index);
                self.len -= 1;
                self.ids[self.len] = 0;
            }
            _ => {}
        }
    }

    /// Keeps only the ids `keep` accepts.
    pub fn retain(&mut self, keep: impl Fn(MaterialId) -> bool) {
        let kept = self.iter().filter(|id| keep(*id)).collect::<Vec<_>>();
        *self = Self::default();
        for id in kept {
            self.set(id, true);
        }
    }
}

impl FromIterator<MaterialId> for AdvancedMaterials {
    fn from_iter<T: IntoIterator<Item = MaterialId>>(ids: T) -> Self {
        let mut set = Self::default();
        for id in ids {
            set.set(id, true);
        }
        set
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationSettings {
    pub grid: bool,
    pub control_polygons: bool,
    pub handles: bool,
    pub accepted_reference: bool,
    pub boundary_conditions: bool,
    pub mesh: bool,
    pub mesh_boundaries: bool,
    pub point_probes: bool,
    pub line_probes: bool,
    pub boundary_probes: bool,
    pub area_probes: bool,
    pub far_field_contour: bool,
    pub probe_labels: bool,
    pub field: bool,
    /// Ceiling on simulated seconds per wall second. The solver is paced to
    /// this rather than to real time; it falls short of it whenever a step
    /// costs more than the frame budget allows.
    pub simulation_speed: f64,
    pub field_gain: f32,
    /// Whether the field's colours are scaled from what is on screen. Off, the
    /// intensity slider is the whole scale, as it was before the field measured
    /// its own.
    pub field_auto_exposure: bool,
    pub vector_overlay: VectorOverlay,
    /// Presentation-only high-pass for complementary-field arrows. The
    /// authoritative canonical field, probes and energy remain untouched.
    pub vector_overlay_ac_coupled: bool,
    pub vector_overlay_density: f32,
    pub vector_overlay_gain: f32,
    pub material_overlay: MaterialOverlay,
    pub material_overlay_opacity: f32,
    pub material_overlay_auto_range: bool,
    pub material_overlay_logarithmic: bool,
    pub material_overlay_manual_min: f64,
    pub material_overlay_manual_max: f64,
    /// What the solver is asked to do, rather than what is drawn. These decide
    /// the mesh a document is simulated on, so a reopened document that meshed
    /// itself differently from the one that was saved is the same surprise as a
    /// changed coefficient would be - and until they were kept here, starting a
    /// session with adaptation off was not expressible at all.
    pub mesh_edge: f64,
    pub adaptation: AdaptationSettings,
    pub grid_scale_filter: bool,
    /// The materials the editor shows in its Advanced view: every law slot and
    /// the numeric effective law, rather than the medium's named values. Each
    /// material keeps its own, so it reads as part of the material, but it is
    /// a way of looking at it: kept with the view, not undone, and never seen
    /// by the solver.
    pub advanced_materials: AdvancedMaterials,
    /// Whether the Advanced view writes each row's effective law with its
    /// expressions evaluated, rather than by the names the author gave them.
    pub law_formula_numbers: bool,
    /// Gate O: paint the integrated field `r` rather than the field, on a
    /// generation that carries a restoring law. Kept with the document so a
    /// scene of kinks or domains opens showing them.
    pub integrated_field: bool,
}

/// The adaptation controls, as the panel shows them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptationSettings {
    pub enabled: bool,
    /// Estimated error of the whole field the adaptation aims for, as a
    /// percentage, held in the units the control shows so the presets are the
    /// round numbers they read as.
    pub accuracy_percent: f64,
    pub elements_per_wavelength: f64,
    pub minimum_edge: f64,
    pub maximum_edge: f64,
}

impl AdaptationSettings {
    pub fn valid(self) -> bool {
        self.accuracy_percent.is_finite()
            && (0.1..=50.0).contains(&self.accuracy_percent)
            && self.elements_per_wavelength.is_finite()
            && (2.0..=64.0).contains(&self.elements_per_wavelength)
            && self.minimum_edge.is_finite()
            && self.maximum_edge.is_finite()
            && self.minimum_edge > 0.0
            && self.minimum_edge <= self.maximum_edge
    }
}

impl Default for AdaptationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            // The medium preset of `AMR_ACCURACY_PRESETS`, which the control
            // reads back by value.
            accuracy_percent: 12.0,
            elements_per_wavelength: 6.0,
            minimum_edge: 0.02,
            maximum_edge: 0.16,
        }
    }
}

impl PresentationSettings {
    pub fn valid(self) -> bool {
        self.simulation_speed.is_finite()
            && (0.02..=2.0).contains(&self.simulation_speed)
            && self.field_gain.is_finite()
            && (0.25..=12.0).contains(&self.field_gain)
            && self.vector_overlay_density.is_finite()
            && (28.0..=120.0).contains(&self.vector_overlay_density)
            && self.vector_overlay_gain.is_finite()
            && (0.1..=5.0).contains(&self.vector_overlay_gain)
            && self.material_overlay_opacity.is_finite()
            && (0.05..=1.0).contains(&self.material_overlay_opacity)
            && self.material_overlay_manual_min.is_finite()
            && self.material_overlay_manual_max.is_finite()
            && self.mesh_edge.is_finite()
            && (0.005..=1.0).contains(&self.mesh_edge)
            && self.adaptation.valid()
    }
}

impl Default for PresentationSettings {
    fn default() -> Self {
        Self {
            grid: true,
            control_polygons: true,
            handles: true,
            accepted_reference: true,
            boundary_conditions: false,
            mesh: false,
            mesh_boundaries: true,
            point_probes: true,
            line_probes: true,
            boundary_probes: true,
            area_probes: true,
            far_field_contour: true,
            probe_labels: true,
            field: true,
            simulation_speed: 1.0,
            field_gain: 2.0,
            field_auto_exposure: true,
            vector_overlay: VectorOverlay::Off,
            vector_overlay_ac_coupled: true,
            vector_overlay_density: 54.0,
            vector_overlay_gain: 1.0,
            material_overlay: MaterialOverlay::Regions,
            material_overlay_opacity: 0.48,
            material_overlay_auto_range: true,
            material_overlay_logarithmic: false,
            material_overlay_manual_min: 0.0,
            material_overlay_manual_max: 1.0,
            mesh_edge: 0.08,
            adaptation: AdaptationSettings::default(),
            grid_scale_filter: true,
            advanced_materials: AdvancedMaterials::default(),
            law_formula_numbers: false,
            integrated_field: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProbeSamplingPreset {
    Low,
    #[default]
    Medium,
    High,
}

impl ProbeSamplingPreset {
    pub const fn spatial_points(self) -> usize {
        match self {
            Self::Low => 32,
            Self::Medium => 64,
            Self::High => 128,
        }
    }

    pub const fn sample_rate(self) -> f64 {
        match self {
            Self::Low => 30.0,
            Self::Medium => 60.0,
            Self::High => 120.0,
        }
    }
}

pub const DEFAULT_FAR_FIELD_INSET: f64 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FarFieldSettings {
    pub enabled: bool,
    pub inset: f64,
}

impl FarFieldSettings {
    pub fn valid(self) -> bool {
        self.inset.is_finite() && self.inset > 0.0
    }
}

impl Default for FarFieldSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            inset: DEFAULT_FAR_FIELD_INSET,
        }
    }
}
