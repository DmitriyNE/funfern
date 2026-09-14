//! Shared persisted presentation and measurement vocabulary.
//!
//! These types are independent of both the retired object-specific editor and
//! the topology editor. Production topology code imports them from here.

use funfern_core::{ElectromagneticPolarization, PhysicsModel};

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
    Property(MaterialProperty),
}

impl MaterialOverlay {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Regions => "Materials",
            Self::Subdomains => "Subdomains",
            Self::Property(property) => property.label(),
        }
    }

    pub const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Regions => "Materials",
            Self::Subdomains => "Subdomains",
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
    /// Modes that actually produce arrows for this physics skin. The mechanical
    /// scalar field has no complementary transverse vector to reconstruct, so
    /// only energy flow is offered there.
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

    /// Maps a stored mode onto one this skin can draw. A scene saved in EM and
    /// reopened as mechanical therefore shows energy flow instead of nothing.
    pub const fn resolved(self, physics: PhysicsModel) -> Self {
        match (self, physics) {
            (Self::ComplementaryField, PhysicsModel::Mechanical) => Self::RelativeEnergyFlow,
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
            // Unreachable: `resolved` turns this into energy flow first.
            (Self::ComplementaryField, PhysicsModel::Mechanical) => "Energy flow",
            (Self::RelativeEnergyFlow, PhysicsModel::Electromagnetic { .. }) => "Poynting flow",
            (Self::RelativeEnergyFlow, PhysicsModel::Mechanical) => "Energy flow",
        }
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
    pub adaptation_target: bool,
    pub point_probes: bool,
    pub line_probes: bool,
    pub boundary_probes: bool,
    pub area_probes: bool,
    pub far_field_contour: bool,
    pub probe_labels: bool,
    pub field: bool,
    pub field_gain: f32,
    pub vector_overlay: VectorOverlay,
    pub vector_overlay_smoothed: bool,
    pub vector_overlay_density: f32,
    pub vector_overlay_gain: f32,
    pub material_overlay: MaterialOverlay,
    pub material_overlay_opacity: f32,
    pub material_overlay_auto_range: bool,
    pub material_overlay_logarithmic: bool,
    pub material_overlay_manual_min: f64,
    pub material_overlay_manual_max: f64,
}

impl PresentationSettings {
    pub fn valid(self) -> bool {
        self.field_gain.is_finite()
            && (0.25..=12.0).contains(&self.field_gain)
            && self.vector_overlay_density.is_finite()
            && (28.0..=120.0).contains(&self.vector_overlay_density)
            && self.vector_overlay_gain.is_finite()
            && (0.1..=5.0).contains(&self.vector_overlay_gain)
            && self.material_overlay_opacity.is_finite()
            && (0.05..=1.0).contains(&self.material_overlay_opacity)
            && self.material_overlay_manual_min.is_finite()
            && self.material_overlay_manual_max.is_finite()
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
            adaptation_target: false,
            point_probes: true,
            line_probes: true,
            boundary_probes: true,
            area_probes: true,
            far_field_contour: true,
            probe_labels: true,
            field: true,
            field_gain: 2.0,
            vector_overlay: VectorOverlay::Off,
            vector_overlay_smoothed: true,
            vector_overlay_density: 54.0,
            vector_overlay_gain: 1.0,
            material_overlay: MaterialOverlay::Regions,
            material_overlay_opacity: 0.48,
            material_overlay_auto_range: true,
            material_overlay_logarithmic: false,
            material_overlay_manual_min: 0.0,
            material_overlay_manual_max: 1.0,
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
