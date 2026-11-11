//! The scenes of the 11 October plan's section 5.2, built with the editor's
//! own API: C8.1's study and checks on the CPU (`tests/edge_cases.rs`) and
//! C8.2's on the device (`ui/e2e/edges.rs`, which includes this file by its
//! path) open the same documents. The app's own paths come through
//! `app_crate`, which each side names: the library itself, or the tests'
//! dependency on it.

// Each side uses its own part of this.
#![allow(dead_code)]

use super::app_crate::document::ProbeSamplingPreset;
use super::app_crate::topology_editor::{
    ClosedCurvePurpose, TopologyAcceptance, TopologyDocument, TopologyEditor, TopologyProbeTarget,
};
use funfern_core::{
    BACKGROUND_REGION, DampingLaw, DomainRect, FieldLaw, LossChannel, Material, MaterialFrame,
    MaterialFrameAttachment, OuterBoundaryCondition, OuterBoundaryConditions, PeriodicCubicSpline,
    Point2, PointSource, RateLaw, RestoringLaw, ScalarField, TimeDrive, TimeSignal, VolumeSource,
};

/// Validates `editor`'s document to an outcome.
pub fn settle(editor: &mut TopologyEditor) {
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != TopologyAcceptance::Pending {
            return;
        }
    }
    panic!("validation did not finish");
}

/// The default box, its point source off and every wall `walls`, edited by
/// `edit`.
pub fn document(
    walls: OuterBoundaryCondition,
    edit: impl FnOnce(&mut TopologyEditor) -> Result<(), String>,
) -> TopologyDocument {
    try_document(walls, edit).unwrap()
}

/// [`document`], an edit the editor refuses returned.
pub fn try_document(
    walls: OuterBoundaryCondition,
    edit: impl FnOnce(&mut TopologyEditor) -> Result<(), String>,
) -> Result<TopologyDocument, String> {
    let mut editor = TopologyEditor::default();
    edit(&mut editor)?;
    settle(&mut editor);
    let mut document = editor.document;
    let walls = OuterBoundaryConditions { sides: [walls; 4] };
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    Ok(document)
}

/// The background's material edited by `author`.
pub fn medium(
    editor: &mut TopologyEditor,
    author: impl FnOnce(&mut Material),
) -> Result<(), String> {
    let mut material = editor.document.model.draft.materials[0].clone();
    author(&mut material);
    editor.update_material(material)
}

pub fn constant(value: f64) -> ScalarField {
    ScalarField::constant(value)
}

pub fn kerr(chi2: f64, bound: Option<f64>) -> FieldLaw {
    FieldLaw::Polynomial {
        chi1: constant(0.0),
        chi2: constant(chi2),
        amplitude_bound: bound.map(constant),
    }
}

/// A law the amplitude sweep runs, and the field amplitude past which it is
/// meant to refuse, if it has one.
pub struct Law {
    pub name: &'static str,
    pub author: Box<dyn Fn(&mut Material)>,
    pub bound: Option<f64>,
}

pub fn laws() -> Vec<Law> {
    let loss = |rate: f64, law: RateLaw| LossChannel {
        base_rate: constant(rate),
        law: DampingLaw {
            rate: law,
            drive: TimeDrive::None,
            gate: None,
        },
    };
    vec![
        Law {
            name: "linear",
            author: Box::new(|_| {}),
            bound: None,
        },
        Law {
            name: "focusing Kerr on the mass",
            author: Box::new(|m| m.mass_law.field = kerr(0.5, None)),
            bound: None,
        },
        Law {
            name: "defocusing Kerr on the mass",
            author: Box::new(|m| m.mass_law.field = kerr(-0.5, Some(0.8))),
            bound: Some(0.8),
        },
        Law {
            name: "focusing Kerr on the stiffness",
            author: Box::new(|m| m.stiffness_law.field = kerr(0.5, None)),
            bound: None,
        },
        Law {
            name: "defocusing Kerr on the stiffness",
            author: Box::new(|m| m.stiffness_law.field = kerr(-0.5, Some(0.8))),
            bound: Some(0.8),
        },
        Law {
            name: "quadratic on the mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Polynomial {
                    chi1: constant(0.3),
                    chi2: constant(0.0),
                    amplitude_bound: Some(constant(1.5)),
                }
            }),
            bound: Some(1.5),
        },
        Law {
            name: "saturable mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Saturable {
                    chi: constant(0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "defocusing saturable mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Saturable {
                    chi: constant(-0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "saturable stiffness",
            author: Box::new(|m| {
                m.stiffness_law.field = FieldLaw::Saturable {
                    chi: constant(0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "Klein-Gordon",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::KleinGordon {
                    omega0: constant(3.0),
                }
            }),
            bound: None,
        },
        Law {
            name: "sine-Gordon",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::SineGordon {
                    omega0: constant(3.0),
                }
            }),
            bound: None,
        },
        Law {
            name: "phi4",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::Phi4 {
                    lambda: constant(4.0),
                    amplitude_bound: constant(2.0),
                }
            }),
            bound: Some(2.0),
        },
        Law {
            name: "van der Pol",
            author: Box::new(move |m| {
                m.restoring = RestoringLaw::KleinGordon {
                    omega0: constant(3.0),
                };
                m.magnetic_loss = Some(loss(
                    0.5,
                    RateLaw::VanDerPol {
                        threshold: constant(0.5),
                        amplitude_bound: constant(2.0),
                    },
                ));
            }),
            bound: Some(2.0),
        },
        Law {
            name: "pumped mass",
            author: Box::new(|m| {
                m.mass_law.drive = TimeDrive::ParametricPump {
                    depth: constant(0.3),
                    frequency_hz: constant(1.0),
                    phase_radians: constant(0.0),
                }
            }),
            bound: None,
        },
    ]
}

/// A disc of radius 0.4 at the middle, of density `density` and stiffness
/// `stiffness` over the background's unit medium.
pub fn inclusion(density: f64, stiffness: f64) -> TopologyDocument {
    document(OuterBoundaryCondition::Reflecting, |editor| {
        let id = editor.add_material()?;
        let mut material = editor
            .document
            .model
            .draft
            .materials
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .clone();
        material.mass_density = constant(density);
        material.stiffness = constant(stiffness);
        editor.update_material(material)?;
        editor.create_closed_curve(
            PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
            ClosedCurvePurpose::Subdomain { material: id },
        )?;
        Ok(())
    })
}

/// The whole box at axis ratio `ratio`, its frame turned 0.5 rad.
pub fn anisotropic(ratio: f64) -> TopologyDocument {
    document(OuterBoundaryCondition::Reflecting, |editor| {
        medium(editor, |m| m.axis_ratio = constant(ratio))?;
        editor.set_region_frame(
            BACKGROUND_REGION,
            MaterialFrame {
                origin: Point2::new(0.0, 0.0),
                angle_radians: 0.5,
                attachment: MaterialFrameAttachment::World,
            },
        )
    })
}

/// The box with `holes` cut, or the editor's refusal.
pub fn holes(holes: Vec<PeriodicCubicSpline>) -> Result<TopologyDocument, String> {
    try_document(OuterBoundaryCondition::Reflecting, move |editor| {
        for spline in holes {
            editor.create_closed_curve(spline, ClosedCurvePurpose::Hole)?;
        }
        Ok(())
    })
}

/// The extreme `x` a closed curve reaches, sampled densely: a B-spline's
/// curve stands inside its controls.
pub fn reach_x(spline: &PeriodicCubicSpline, sign: f64) -> f64 {
    let period = spline.period();
    (0..8192)
        .map(|i| sign * spline.evaluate(period * i as f64 / 8192.0).x)
        .fold(f64::NEG_INFINITY, f64::max)
        * sign
}

/// A rounded hole of radius `radius` whose curve comes `gap` from the line
/// `x = wall`, on the side `sign` points to.
pub fn disc_off(wall: f64, sign: f64, gap: f64, radius: f64) -> PeriodicCubicSpline {
    let probe = PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), radius);
    let reach = reach_x(&probe, sign).abs();
    PeriodicCubicSpline::rounded(Point2::new(wall - sign * (reach + gap), 0.0), radius)
}

/// A polygon hole: the square `[0.4, 1 − gap] × [−0.3, 0.3]`, a channel
/// `gap` wide along the right wall.
pub fn channel(gap: f64) -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(0.4, -0.3),
        Point2::new(1.0 - gap, -0.3),
        Point2::new(1.0 - gap, 0.3),
        Point2::new(0.4, 0.3),
    ])
    .unwrap()
}

/// A pentagonal hole one of whose edges is `length` long.
pub fn short_edge(length: f64) -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(-0.3, -0.25),
        Point2::new(0.3, -0.25),
        Point2::new(0.35, 0.1),
        Point2::new(0.0, 0.3),
        Point2::new(-0.35, 0.1 + length),
        Point2::new(-0.35, 0.1),
    ])
    .unwrap()
}

/// A rounded hole's controls with the first two drawn `gap` apart.
pub fn pinched(gap: f64) -> PeriodicCubicSpline {
    let mut controls = (0..8)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 8.0;
            Point2::new(0.3 * angle.cos(), 0.3 * angle.sin())
        })
        .collect::<Vec<_>>();
    let (a, b) = (controls[0], controls[1]);
    let middle = (a + b) * 0.5;
    let along = (b - a) * (1.0 / (b - a).norm());
    controls[0] = middle - along * (0.5 * gap);
    controls[1] = middle + along * (0.5 * gap);
    PeriodicCubicSpline::uniform(controls).unwrap()
}

/// The empty-resource matrix's scene: `source` none, "point" or "volume";
/// `probe` none, "point", "segment" or "disc"; every wall `walls`; `history`
/// none, "loss" or "restoring"; and the mass pumped where `driven`.
pub fn empty_case(
    source: &str,
    probe: &str,
    walls: OuterBoundaryCondition,
    history: &str,
    driven: bool,
) -> TopologyDocument {
    document(walls, |editor| {
        match source {
            "point" => editor.set_point_source(PointSource {
                enabled: true,
                ..PointSource::default()
            })?,
            "volume" => editor.set_volume_source(
                BACKGROUND_REGION,
                Some(VolumeSource {
                    region: BACKGROUND_REGION,
                    enabled: true,
                    profile: ScalarField::formula("exp(-((x - 0.3)^2 + y^2) / 0.05)")
                        .map_err(|e| e.to_string())?,
                    parameters: vec![],
                    signal: TimeSignal::harmonic(0.0, 1.0, 1.5, 0.0),
                }),
            )?,
            _ => {}
        }
        let target = match probe {
            "point" => Some(TopologyProbeTarget::Point(Point2::new(0.2, 0.1))),
            "segment" => Some(TopologyProbeTarget::Segment {
                start: Point2::new(-0.5, -0.2),
                end: Point2::new(0.5, 0.3),
                preset: ProbeSamplingPreset::Medium,
            }),
            "disc" => Some(TopologyProbeTarget::AreaDisk {
                center: Point2::new(0.1, -0.2),
                radius: 0.3,
            }),
            _ => None,
        };
        if let Some(target) = target {
            editor.create_probe("probe".into(), [200, 80, 80], target)?;
        }
        medium(editor, |m| {
            match history {
                "loss" => {
                    m.magnetic_loss = Some(LossChannel {
                        base_rate: constant(0.2),
                        law: DampingLaw {
                            rate: RateLaw::Constant,
                            drive: TimeDrive::None,
                            gate: None,
                        },
                    })
                }
                "restoring" => {
                    m.restoring = RestoringLaw::KleinGordon {
                        omega0: constant(2.0),
                    }
                }
                _ => {}
            }
            if driven {
                m.mass_law.drive = TimeDrive::ParametricPump {
                    depth: constant(0.2),
                    frequency_hz: constant(1.0),
                    phase_radians: constant(0.0),
                };
            }
        })
    })
}

/// The box's domain made `width` by `2 × half_height`, centred.
pub fn sized(document: &mut TopologyDocument, width: f64, half_height: f64) {
    let domain = DomainRect::new(-0.5 * width, 0.5 * width, -half_height, half_height);
    for model in [&mut document.model.draft, &mut document.model.accepted] {
        model.geometry.domain = domain;
    }
}

/// A square hole 0.06 across at the middle, which a box as narrow as 0.1
/// holds: a mesh with one hole has an even node count.
pub fn small_square() -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(-0.03, -0.03),
        Point2::new(0.03, -0.03),
        Point2::new(0.03, 0.03),
        Point2::new(-0.03, 0.03),
    ])
    .unwrap()
}
