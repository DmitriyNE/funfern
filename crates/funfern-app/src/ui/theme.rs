//! The palette and the colour ramps: which colour stands for which boundary
//! condition, material property, field level or adaptation target.

use crate::material_overlay::MaterialProperty;
use bevy_egui::egui;
use bevy_egui::egui::Color32;
use funfern_core::*;

pub(super) const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
pub(super) const SELECT: Color32 = Color32::from_rgb(72, 166, 255);
pub(super) const RED: Color32 = Color32::from_rgb(255, 106, 123);
pub(super) const GOLD: Color32 = Color32::from_rgb(248, 196, 112);

pub(super) fn face_condition_color(condition: FaceBoundaryCondition) -> Color32 {
    match condition {
        FaceBoundaryCondition::Reflecting => Color32::from_rgb(184, 201, 211),
        FaceBoundaryCondition::Impedance { .. } => Color32::from_rgb(91, 220, 194),
        FaceBoundaryCondition::SecondOrderOutgoing => Color32::from_rgb(87, 174, 255),
        FaceBoundaryCondition::ElectricWall => Color32::from_rgb(255, 126, 141),
        FaceBoundaryCondition::MagneticWall => Color32::from_rgb(188, 143, 255),
        FaceBoundaryCondition::Neumann { .. } => Color32::from_rgb(248, 196, 112),
        FaceBoundaryCondition::Dirichlet { .. } => Color32::from_rgb(255, 139, 84),
    }
}

pub(super) fn outer_condition_color(condition: OuterBoundaryCondition) -> Color32 {
    match condition {
        OuterBoundaryCondition::Reflecting => {
            face_condition_color(FaceBoundaryCondition::Reflecting)
        }
        OuterBoundaryCondition::FirstOrderOutgoing => {
            face_condition_color(FaceBoundaryCondition::Impedance { ratio: 1.0 })
        }
        OuterBoundaryCondition::SecondOrderOutgoing => {
            face_condition_color(FaceBoundaryCondition::SecondOrderOutgoing)
        }
        OuterBoundaryCondition::ElectricWall => {
            face_condition_color(FaceBoundaryCondition::ElectricWall)
        }
        OuterBoundaryCondition::MagneticWall => {
            face_condition_color(FaceBoundaryCondition::MagneticWall)
        }
        OuterBoundaryCondition::Neumann { signal } => {
            face_condition_color(FaceBoundaryCondition::Neumann { signal })
        }
        OuterBoundaryCondition::Dirichlet { signal } => {
            face_condition_color(FaceBoundaryCondition::Dirichlet { signal })
        }
    }
}

/// Diverging blue/red ramp for a signed waterfall cell, or a single-sided ramp
/// for a non-negative quantity such as energy density.
pub(super) fn waterfall_color(normalized: f32, single_sided: bool) -> Color32 {
    if single_sided {
        let amount = normalized.max(0.0);
        Color32::from_rgb(
            (22.0 + amount * 233.0) as u8,
            (35.0 + amount * 155.0) as u8,
            (55.0 + amount * 55.0) as u8,
        )
    } else if normalized >= 0.0 {
        Color32::from_rgb(
            (30.0 + normalized * 225.0) as u8,
            (45.0 + normalized * 90.0) as u8,
            (60.0 + normalized * 45.0) as u8,
        )
    } else {
        let amount = -normalized;
        Color32::from_rgb(
            (30.0 + amount * 35.0) as u8,
            (45.0 + amount * 65.0) as u8,
            (60.0 + amount * 195.0) as u8,
        )
    }
}

/// `scaled` is the node's value already divided by whatever scale is in force —
/// the exposure's reference when it is on, the intensity slider alone when it is
/// not — so this only has to decide a colour.
#[cfg(test)]
pub(super) fn field_color(scaled: f32, under: Color32) -> Color32 {
    let value = scaled.tanh();
    let target = if value >= 0.0 {
        Color32::from_rgb(244, 105, 122)
    } else {
        Color32::from_rgb(63, 144, 239)
    };
    let amount = value.abs();
    let base = if under == Color32::TRANSPARENT {
        Color32::from_rgb(16, 23, 31)
    } else {
        under
    };
    Color32::from_rgb(
        egui::lerp(base.r() as f32..=target.r() as f32, amount) as u8,
        egui::lerp(base.g() as f32..=target.g() as f32, amount) as u8,
        egui::lerp(base.b() as f32..=target.b() as f32, amount) as u8,
    )
}

#[cfg(test)]
pub(super) fn field_color_over_overlay(scaled: f32) -> Color32 {
    let value = if scaled.is_finite() {
        scaled.tanh()
    } else {
        0.0
    };
    let target = if value >= 0.0 {
        [244, 105, 122]
    } else {
        [63, 144, 239]
    };
    Color32::from_rgba_unmultiplied(
        target[0],
        target[1],
        target[2],
        (value.abs() * 220.0).round() as u8,
    )
}

/// Categorical colour for one subdomain, taken by the region's position in the
/// authored draft list so neighbouring faces sharing a material still read
/// apart. Every caller must pass the same scene — the draft — or the panel and
/// the viewport disagree about which colour belongs to which region.
pub(super) fn subdomain_color(scene: &TopologyScene, region: RegionId, opacity: f32) -> Color32 {
    const PALETTE: [[u8; 3]; 10] = [
        [91, 220, 194],
        [248, 196, 112],
        [174, 126, 241],
        [255, 106, 123],
        [72, 166, 255],
        [126, 217, 87],
        [255, 154, 70],
        [236, 130, 200],
        [110, 198, 233],
        [204, 194, 108],
    ];
    let Some(index) = scene
        .regions
        .iter()
        .position(|candidate| candidate.id == region)
    else {
        return Color32::TRANSPARENT;
    };
    let [red, green, blue] = PALETTE[index % PALETTE.len()];
    Color32::from_rgba_unmultiplied(red, green, blue, (opacity * 210.0) as u8)
}

pub(super) fn material_property_color(fraction: f32, alpha: u8) -> Color32 {
    const STOPS: [(f32, [u8; 3]); 5] = [
        (0.0, [20, 34, 69]),
        (0.25, [42, 91, 132]),
        (0.5, [55, 168, 154]),
        (0.75, [184, 205, 104]),
        (1.0, [255, 211, 103]),
    ];
    let value = fraction.clamp(0.0, 1.0);
    let (left, right) = STOPS
        .windows(2)
        .find_map(|pair| (value <= pair[1].0).then_some((pair[0], pair[1])))
        .unwrap_or((STOPS[3], STOPS[4]));
    let t = ((value - left.0) / (right.0 - left.0)).clamp(0.0, 1.0);
    let channel = |index: usize| {
        (left.1[index] as f32 + t * (right.1[index] as f32 - left.1[index] as f32)).round() as u8
    };
    Color32::from_rgba_unmultiplied(channel(0), channel(1), channel(2), alpha)
}

pub(super) fn overlay_property_color(
    property: MaterialProperty,
    fraction: f32,
    alpha: u8,
) -> Color32 {
    if property != MaterialProperty::VolumeSource {
        return material_property_color(fraction, alpha);
    }
    const STOPS: [(f32, [u8; 3]); 3] = [
        (0.0, [63, 144, 239]),
        (0.5, [24, 32, 39]),
        (1.0, [244, 105, 122]),
    ];
    let value = fraction.clamp(0.0, 1.0);
    let (left, right) = if value <= 0.5 {
        (STOPS[0], STOPS[1])
    } else {
        (STOPS[1], STOPS[2])
    };
    let t = ((value - left.0) / (right.0 - left.0)).clamp(0.0, 1.0);
    let channel = |index: usize| {
        (left.1[index] as f32 + t * (right.1[index] as f32 - left.1[index] as f32)).round() as u8
    };
    Color32::from_rgba_unmultiplied(channel(0), channel(1), channel(2), alpha)
}

pub(super) fn amr_target_color(fraction: f32, alpha: u8) -> Color32 {
    let fraction = fraction.clamp(0.0, 1.0);
    let fine = [71.0, 144.0, 232.0];
    let coarse = [246.0, 183.0, 92.0];
    Color32::from_rgba_unmultiplied(
        egui::lerp(fine[0]..=coarse[0], fraction) as u8,
        egui::lerp(fine[1]..=coarse[1], fraction) as u8,
        egui::lerp(fine[2]..=coarse[2], fraction) as u8,
        alpha,
    )
}
