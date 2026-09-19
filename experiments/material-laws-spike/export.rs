use funfern_core::*;

// Structured fixtures make parent/child and split support exactly reproducible.
// Every basis, mass/stiffness and radiation operator below comes from clean core.
fn mesh(n: usize, split: bool) -> TriMesh {
    let mut vertices = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            vertices.push(MeshVertex {
                point: Point2::new(
                    -1.0 + 2.0 * i as f64 / n as f64,
                    -1.0 + 2.0 * j as f64 / n as f64,
                ),
                boundary: None,
                trace: None,
            });
        }
    }
    let mut duplicates = Vec::new();
    if split {
        for j in 0..=n {
            duplicates.push(vertices.len());
            vertices.push(vertices[j * (n + 1) + n / 2]);
        }
    }
    let index = |i: usize, j: usize, right: bool| {
        if split && i == n / 2 && right {
            duplicates[j]
        } else {
            j * (n + 1) + i
        }
    };
    let mut triangles = Vec::new();
    let mut boundary_edges = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let r = i >= n / 2;
            let a = index(i, j, r);
            let b = index(i + 1, j, r);
            let c = index(i + 1, j + 1, r);
            let d = index(i, j + 1, r);
            triangles.push(MeshTriangle {
                vertices: [a, b, c],
                region: BACKGROUND_REGION,
            });
            triangles.push(MeshTriangle {
                vertices: [a, c, d],
                region: BACKGROUND_REGION,
            });
            for (yes, pair, side) in [
                (j == 0, [a, b], OuterSide::Bottom),
                (i == n - 1, [b, c], OuterSide::Right),
                (j == n - 1, [c, d], OuterSide::Top),
                (i == 0, [d, a], OuterSide::Left),
            ] {
                if yes {
                    boundary_edges.push(BoundaryEdge {
                        vertices: pair,
                        label: BoundaryLabel::Outer(side),
                        parameters: [0.0, 1.0],
                    });
                }
            }
        }
    }
    TriMesh {
        geometry_revision: 1,
        mesh_revision: n as u64,
        vertices,
        triangles,
        boundary_edges,
        quality: MeshQuality {
            minimum_angle_degrees: 45.0,
            maximum_edge_length: 2.0_f64.sqrt() * 2.0 / n as f64,
        },
        requested_sizes: vec![],
    }
}

// Optional curved-geometry smoke fixture: polygonal annulus, unit isotropic
// coefficients. Outer-side labels select the absorber; their fixed normals do
// not affect unit isotropic impedance. This is not the ignored topology test.
fn annulus() -> TriMesh {
    let sectors = 24;
    let rings = 3;
    let mut vertices = Vec::new();
    for j in 0..=rings {
        let radius = 0.35 + 0.65 * j as f64 / rings as f64;
        for i in 0..sectors {
            let angle = std::f64::consts::TAU * i as f64 / sectors as f64;
            vertices.push(MeshVertex {
                point: Point2::new(radius * angle.cos(), radius * angle.sin()),
                boundary: None,
                trace: None,
            });
        }
    }
    let mut triangles = Vec::new();
    for j in 0..rings {
        for i in 0..sectors {
            let next = (i + 1) % sectors;
            let a = j * sectors + i;
            let b = (j + 1) * sectors + i;
            let c = (j + 1) * sectors + next;
            let d = j * sectors + next;
            triangles.push(MeshTriangle {
                vertices: [a, b, c],
                region: BACKGROUND_REGION,
            });
            triangles.push(MeshTriangle {
                vertices: [a, c, d],
                region: BACKGROUND_REGION,
            });
        }
    }
    let mut boundary_edges = Vec::new();
    for (j, side) in [(0, OuterSide::Left), (rings, OuterSide::Right)] {
        for i in 0..sectors {
            boundary_edges.push(BoundaryEdge {
                vertices: [j * sectors + i, j * sectors + (i + 1) % sectors],
                label: BoundaryLabel::Outer(side),
                parameters: [0., 1.],
            });
        }
    }
    TriMesh {
        geometry_revision: 1,
        mesh_revision: 1,
        vertices,
        triangles,
        boundary_edges,
        quality: MeshQuality {
            minimum_angle_degrees: 10.,
            maximum_edge_length: 0.4,
        },
        requested_sizes: vec![],
    }
}

fn main() {
    let a = 0.445948490915965;
    let b = 0.108103018168070;
    let c = 0.091576213509771;
    let d = 0.816847572980459;
    let qs = [
        ([a, a, b], 0.223381589678011),
        ([a, b, a], 0.223381589678011),
        ([b, a, a], 0.223381589678011),
        ([c, c, d], 0.109951743655322),
        ([c, d, c], 0.109951743655322),
        ([d, c, c], 0.109951743655322),
    ];
    print!("{{");
    let mut cases = vec![
        ("coarse", 4, false),
        ("fine", 8, false),
        ("packet", 16, false),
        ("split", 4, true),
    ];
    if std::env::args().any(|arg| arg == "--curved") {
        cases.push(("curved", 24, false));
    }
    for (case, (name, n, split)) in cases.into_iter().enumerate() {
        let mesh = if name == "curved" {
            annulus()
        } else {
            mesh(n, split)
        };
        let op = QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        let outgoing = QuadraticWaveOperator::assemble_with_boundary(
            &mesh,
            WaveCoefficients::default(),
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let mut grad = Vec::new();
        let mut points = Vec::new();
        let mut weights = Vec::new();
        let mut triangles = Vec::new();
        for tri in &mesh.triangles {
            let p = tri.vertices.map(|i| mesh.vertices[i].point);
            triangles.push(p.map(|p| [p.x, p.y]));
            let det = (p[1] - p[0]).cross(p[2] - p[0]);
            let g = [
                Point2::new(p[1].y - p[2].y, p[2].x - p[1].x) / det,
                Point2::new(p[2].y - p[0].y, p[0].x - p[2].x) / det,
                Point2::new(p[0].y - p[1].y, p[1].x - p[0].x) / det,
            ];
            let mut eg = Vec::new();
            let mut ep = Vec::new();
            let mut ew = Vec::new();
            for (q, w) in qs {
                eg.push(enriched_quadratic_basis_gradients(q, g).map(|g| [-g.y, g.x]));
                let x = p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
                ep.push([x.x, x.y]);
                ew.push(det * 0.5 * w);
            }
            grad.push(eg);
            points.push(ep);
            weights.push(ew);
        }
        if case > 0 {
            print!(",");
        }
        print!(
            "\"{name}\":{{\"n\":{n},\"nodes\":{:?},\"elements\":{:?},\"triangles\":{:?},\"c\":{:?},\"qpoints\":{:?},\"w\":{:?},\"mass\":{:?},\"rows\":{:?},\"cols\":{:?},\"k\":{:?},\"boundary_d\":{:?},\"aux\":{:?},\"aux_rows\":{:?},\"aux_cols\":{:?},\"dt_max\":{},\"old_estimated_bytes\":{}}}",
            op.node_points()
                .iter()
                .map(|p| [p.x, p.y])
                .collect::<Vec<_>>(),
            op.element_nodes(),
            triangles,
            grad,
            points,
            weights,
            op.lumped_mass(),
            op.row_offsets(),
            op.columns(),
            op.stiffness_values(),
            outgoing.lumped_damping(),
            outgoing.auxiliary_stiffness_values(),
            outgoing.row_offsets(),
            outgoing.columns(),
            op.maximum_time_step(),
            op.estimated_gpu_bytes()
        );
    }
    println!("}}");
}
