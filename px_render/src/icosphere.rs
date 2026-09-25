use crate::mesh::Mesh;

type V3 = [f32; 3];

const ZERO: V3 = [0.0, 0.0, 0.0];

const EDGES: usize = 30;

#[rustfmt::skip]
const INITIAL_POINTS: [V3; 12] = [
    [0.0, 1.0, 0.0],
    [0.89442719099991585541, 0.44721359549995792770, 0.00000000000000000000],
    [0.27639320225002106390, 0.44721359549995792770, 0.85065080835203987775],
    [-0.72360679774997882507, 0.44721359549995792770, 0.52573111211913370333],
    [-0.72360679774997904712, 0.44721359549995792770, -0.52573111211913348129],
    [0.27639320225002084186, 0.44721359549995792770, -0.85065080835203998877],
    [0.72360679774997871405, -0.44721359549995792770, -0.52573111211913392538],
    [0.72360679774997904712, -0.44721359549995792770, 0.52573111211913337026],
    [-0.27639320225002073084, -0.44721359549995792770, 0.85065080835203998877],
    [-0.89442719099991585541, -0.44721359549995792770, 0.00000000000000000000],
    [-0.27639320225002139697, -0.44721359549995792770, -0.85065080835203976672],
    [0.0, -1.0, 0.0],
];

#[rustfmt::skip]
const TRIANGLES: [(u32, u32, u32, usize, usize, usize); 20] = [
    (0, 2, 1, 0, 5, 4),
    (0, 3, 2, 1, 6, 0),
    (0, 4, 3, 2, 7, 1),
    (0, 5, 4, 3, 8, 2),
    (0, 1, 5, 4, 9, 3),
    (5, 1, 6, 9, 10, 15),
    (1, 2, 7, 5, 11, 16),
    (2, 3, 8, 6, 12, 17),
    (3, 4, 9, 7, 13, 18),
    (4, 5, 10, 8, 14, 19),
    (5, 6, 10, 15, 20, 14),
    (1, 7, 6, 16, 21, 10),
    (2, 8, 7, 17, 22, 11),
    (3, 9, 8, 18, 23, 12),
    (4, 10, 9, 19, 24, 13),
    (10, 6, 11, 20, 26, 25),
    (6, 7, 11, 21, 27, 26),
    (7, 8, 11, 22, 28, 27),
    (8, 9, 11, 23, 29, 28),
    (9, 10, 11, 24, 25, 29),
];

#[inline]
fn vadd(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn vmul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn vdot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn geometric_slerp_half(a: V3, b: V3) -> V3 {
    vmul(vadd(a, b), (2.0 * (1.0 + vdot(a, b))).sqrt().recip())
}

fn geometric_slerp_multiple(a: V3, b: V3, indices: &[u32], points: &mut [V3]) {
    let angle = vdot(a, b).acos();
    let sin = angle.sin().recip();

    for (percent, index) in indices.iter().enumerate() {
        let percent = (percent + 1) as f32 / (indices.len() + 1) as f32;

        points[*index as usize] = vadd(
            vmul(a, ((1.0 - percent) * angle).sin() * sin),
            vmul(b, (percent * angle).sin() * sin),
        );
    }
}

#[derive(Clone, Copy)]
enum Slice<'a> {
    Forward(&'a [u32]),
    Backward(&'a [u32]),
}

impl Slice<'_> {
    fn len(&self) -> usize {
        match self {
            Slice::Forward(x) | Slice::Backward(x) => x.len(),
        }
    }

    fn at(&self, idx: usize) -> u32 {
        match self {
            Slice::Forward(x) => x[idx],
            Slice::Backward(x) => x[(x.len() - 1) - idx],
        }
    }
}

struct Edge {
    points: Vec<u32>,
    done: bool,
}

impl Default for Edge {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            done: true,
        }
    }
}

impl Edge {
    fn subdivide_n_times(&mut self, n: usize, points: &mut usize) {
        for _ in 0..n {
            self.points.push(*points as u32);
            *points += 1;
        }
    }
}

#[derive(Debug)]
enum TriangleContents {
    None,
    One(u32),
    Three {
        a: u32,
        b: u32,
        c: u32,
    },
    Six {
        a: u32,
        b: u32,
        c: u32,
        ab: u32,
        bc: u32,
        ca: u32,
    },
    More {
        a: u32,
        b: u32,
        c: u32,
        sides: Vec<u32>,
        my_side_length: u32,
        contents: Box<TriangleContents>,
    },
}

impl TriangleContents {
    fn none() -> Self {
        Self::None
    }

    fn one(points: &mut usize) -> Self {
        let index = *points as u32;
        *points += 1;
        TriangleContents::One(index)
    }

    fn calculate_one(&self, ab: Slice<'_>, bc: Slice<'_>, points: &mut [V3]) {
        assert_eq!(ab.len(), bc.len());
        assert_eq!(ab.len(), 2);

        match self {
            TriangleContents::One(idx) => {
                let p1 = points[ab.at(0) as usize];
                let p2 = points[bc.at(1) as usize];

                points[*idx as usize] = geometric_slerp_half(p1, p2);
            }
            _ => panic!("Did not find One variant."),
        }
    }

    fn three(&mut self, points: &mut usize) {
        match self {
            &mut TriangleContents::One(x) => {
                *points += 2;

                *self = TriangleContents::Three {
                    a: x,
                    b: *points as u32 - 2,
                    c: *points as u32 - 1,
                };
            }
            _ => panic!("Self is {:?} while it should be One", self),
        }
    }

    fn calculate_three(&self, ab: Slice<'_>, bc: Slice<'_>, ca: Slice<'_>, points: &mut [V3]) {
        assert_eq!(ab.len(), bc.len());
        assert_eq!(ab.len(), ca.len());
        assert_eq!(ab.len(), 3);

        match self {
            TriangleContents::Three { a, b, c } => {
                let ab = points[ab.at(1) as usize];
                let bc = points[bc.at(1) as usize];
                let ca = points[ca.at(1) as usize];

                let a_val = geometric_slerp_half(ab, ca);
                let b_val = geometric_slerp_half(bc, ab);
                let c_val = geometric_slerp_half(ca, bc);

                points[*a as usize] = a_val;
                points[*b as usize] = b_val;
                points[*c as usize] = c_val;
            }
            _ => panic!("Did not find Three variant."),
        }
    }

    fn six(&mut self, points: &mut usize) {
        match self {
            &mut TriangleContents::Three {
                a: a_index,
                b: b_index,
                c: c_index,
            } => {
                *points += 3;

                *self = TriangleContents::Six {
                    a: a_index,
                    b: b_index,
                    c: c_index,
                    ab: *points as u32 - 3,
                    bc: *points as u32 - 2,
                    ca: *points as u32 - 1,
                };
            }
            _ => panic!("Found {:?} whereas a Three was expected", self),
        }
    }

    fn calculate_six(&self, ab: Slice<'_>, bc: Slice<'_>, ca: Slice<'_>, points: &mut [V3]) {
        assert_eq!(ab.len(), bc.len());
        assert_eq!(ab.len(), ca.len());
        assert_eq!(ab.len(), 4);

        match self {
            TriangleContents::Six {
                a: a_index,
                b: b_index,
                c: c_index,
                ab: ab_index,
                bc: bc_index,
                ca: ca_index,
            } => {
                let aba = points[ab.at(1) as usize];
                let abb = points[ab.at(2) as usize];
                let bcb = points[bc.at(1) as usize];
                let bcc = points[bc.at(2) as usize];
                let cac = points[ca.at(1) as usize];
                let caa = points[ca.at(2) as usize];

                let a = geometric_slerp_half(aba, caa);
                let b = geometric_slerp_half(abb, bcb);
                let c = geometric_slerp_half(bcc, cac);

                let ab = geometric_slerp_half(a, b);
                let bc = geometric_slerp_half(b, c);
                let ca = geometric_slerp_half(c, a);

                points[*a_index as usize] = a;
                points[*b_index as usize] = b;
                points[*c_index as usize] = c;
                points[*ab_index as usize] = ab;
                points[*bc_index as usize] = bc;
                points[*ca_index as usize] = ca;
            }
            _ => panic!("Found {:?} whereas a Three was expected", self),
        }
    }

    fn subdivide(&mut self, points: &mut usize) {
        match self {
            TriangleContents::None => *self = Self::one(points),
            TriangleContents::One(_) => self.three(points),
            TriangleContents::Three { .. } => self.six(points),
            &mut TriangleContents::Six {
                a,
                b,
                c,
                ab: ab_idx,
                bc: bc_idx,
                ca: ca_idx,
            } => {
                *self = TriangleContents::More {
                    a,
                    b,
                    c,
                    sides: vec![ab_idx, bc_idx, ca_idx],
                    my_side_length: 1,
                    contents: Box::new(Self::none()),
                };
                self.subdivide(points);
            }
            TriangleContents::More {
                sides,
                contents,
                my_side_length,
                ..
            } => {
                *points += 3;
                let len = *points as u32;
                sides.extend_from_slice(&[len - 3, len - 2, len - 1]);
                *my_side_length += 1;

                contents.subdivide(points);
            }
        }
    }

    fn calculate(&mut self, ab: Slice<'_>, bc: Slice<'_>, ca: Slice<'_>, points: &mut [V3]) {
        assert_eq!(ab.len(), bc.len());
        assert_eq!(ab.len(), ca.len());
        assert!(ab.len() >= 2);

        match self {
            TriangleContents::None => panic!(),
            TriangleContents::One(_) => self.calculate_one(ab, bc, points),
            TriangleContents::Three { .. } => self.calculate_three(ab, bc, ca, points),
            TriangleContents::Six { .. } => self.calculate_six(ab, bc, ca, points),
            &mut TriangleContents::More {
                a: a_idx,
                b: b_idx,
                c: c_idx,
                ref mut sides,
                ref mut contents,
                ref mut my_side_length,
            } => {
                let side_length = *my_side_length as usize;

                let outer_len = ab.len();

                let aba = points[ab.at(1) as usize];
                let abb = points[ab.at(outer_len - 2) as usize];
                let bcb = points[bc.at(1) as usize];
                let bcc = points[bc.at(outer_len - 2) as usize];
                let cac = points[ca.at(1) as usize];
                let caa = points[ca.at(outer_len - 2) as usize];

                points[a_idx as usize] = geometric_slerp_half(aba, caa);
                points[b_idx as usize] = geometric_slerp_half(abb, bcb);
                points[c_idx as usize] = geometric_slerp_half(bcc, cac);

                let ab = &sides[0..side_length];
                let bc = &sides[side_length..side_length * 2];
                let ca = &sides[side_length * 2..];

                geometric_slerp_multiple(
                    points[a_idx as usize],
                    points[b_idx as usize],
                    ab,
                    points,
                );
                geometric_slerp_multiple(
                    points[b_idx as usize],
                    points[c_idx as usize],
                    bc,
                    points,
                );
                geometric_slerp_multiple(
                    points[c_idx as usize],
                    points[a_idx as usize],
                    ca,
                    points,
                );

                contents.calculate(
                    Slice::Forward(ab),
                    Slice::Forward(bc),
                    Slice::Forward(ca),
                    points,
                );
            }
        }
    }

    fn idx_ab(&self, idx: usize) -> u32 {
        match self {
            TriangleContents::None => panic!("Invalid Index, len is 0, but got {}", idx),
            TriangleContents::One(x) => {
                if idx != 0 {
                    panic!("Invalid Index, len is 1, but got {}", idx);
                } else {
                    *x
                }
            }
            TriangleContents::Three { a, b, .. } => *[a, b][idx],
            TriangleContents::Six { a, b, ab, .. } => *[a, ab, b][idx],
            TriangleContents::More {
                a,
                b,
                sides,
                my_side_length,
                ..
            } => match idx {
                0 => *a,
                x if (1..(*my_side_length as usize + 1)).contains(&x) => sides[x - 1],
                x if x == *my_side_length as usize + 1 => *b,
                _ => panic!(
                    "Invalid Index, len is {}, but got {}",
                    my_side_length + 2,
                    idx
                ),
            },
        }
    }

    fn idx_bc(&self, idx: usize) -> u32 {
        match self {
            TriangleContents::None => panic!("Invalid Index, len is 0, but got {}", idx),
            TriangleContents::One(x) => {
                if idx != 0 {
                    panic!("Invalid Index, len is 1, but got {}", idx);
                } else {
                    *x
                }
            }
            TriangleContents::Three { c, b, .. } => *[b, c][idx],
            TriangleContents::Six { b, c, bc, .. } => *[b, bc, c][idx],
            TriangleContents::More {
                b,
                c,
                sides,
                my_side_length,
                ..
            } => match idx {
                0 => *b,
                x if (1..(*my_side_length as usize + 1)).contains(&x) => {
                    sides[*my_side_length as usize + (x - 1)]
                }
                x if x == *my_side_length as usize + 1 => *c,
                _ => panic!(
                    "Invalid Index, len is {}, but got {}",
                    my_side_length + 2,
                    idx
                ),
            },
        }
    }

    fn idx_ca(&self, idx: usize) -> u32 {
        match self {
            TriangleContents::None => panic!("Invalid Index, len is 0, but got {}", idx),
            TriangleContents::One(x) => {
                if idx != 0 {
                    panic!("Invalid Index, len is 1, but got {}", idx);
                } else {
                    *x
                }
            }
            TriangleContents::Three { c, a, .. } => *[c, a][idx],
            TriangleContents::Six { c, a, ca, .. } => *[c, ca, a][idx],
            TriangleContents::More {
                c,
                a,
                sides,
                my_side_length,
                ..
            } => match idx {
                0 => *c,
                x if (1..(*my_side_length as usize + 1)).contains(&x) => {
                    sides[*my_side_length as usize * 2 + x - 1]
                }
                x if x == *my_side_length as usize + 1 => *a,
                _ => panic!(
                    "Invalid Index, len is {}, but got {}",
                    my_side_length + 2,
                    idx
                ),
            },
        }
    }

    fn add_indices(&self, buffer: &mut Vec<u32>) {
        match self {
            TriangleContents::None | TriangleContents::One(_) => {}
            &TriangleContents::Three { a, b, c } => buffer.extend_from_slice(&[a, b, c]),
            &TriangleContents::Six {
                a,
                b,
                c,
                ab,
                bc,
                ca,
            } => {
                buffer.extend_from_slice(&[a, ab, ca]);
                buffer.extend_from_slice(&[ab, b, bc]);
                buffer.extend_from_slice(&[bc, c, ca]);

                buffer.extend_from_slice(&[ab, bc, ca]);
            }
            &TriangleContents::More {
                a,
                b,
                c,
                ref sides,
                my_side_length,
                ref contents,
            } => {
                let my_side_length = my_side_length as usize;
                let ab = &sides[0..my_side_length];
                let bc = &sides[my_side_length..my_side_length * 2];
                let ca = &sides[my_side_length * 2..];

                add_indices_triangular(
                    a,
                    b,
                    c,
                    Slice::Forward(ab),
                    Slice::Forward(bc),
                    Slice::Forward(ca),
                    contents,
                    buffer,
                );
                contents.add_indices(buffer);
            }
        }
    }
}

struct Triangle {
    a: u32,
    b: u32,
    c: u32,
    ab_edge: usize,
    bc_edge: usize,
    ca_edge: usize,
    ab_forward: bool,
    bc_forward: bool,
    ca_forward: bool,
    contents: TriangleContents,
}

impl Triangle {
    fn calculate_edges(&mut self, edges: &mut [Edge], points: &mut [V3]) -> usize {
        let mut divide = |p1: u32, p2: u32, edge_idx: usize, forward: &mut bool| {
            if !edges[edge_idx].done {
                geometric_slerp_multiple(
                    points[p1 as usize],
                    points[p2 as usize],
                    &edges[edge_idx].points,
                    points,
                );

                edges[edge_idx].done = true;
                *forward = true;
            } else {
                *forward = false;
            }
        };

        divide(self.a, self.b, self.ab_edge, &mut self.ab_forward);
        divide(self.b, self.c, self.bc_edge, &mut self.bc_forward);
        divide(self.c, self.a, self.ca_edge, &mut self.ca_forward);

        edges[self.ab_edge].points.len()
    }

    fn subdivide(&mut self, points: &mut usize, subdivision_level: usize) {
        if subdivision_level >= 1 {
            self.contents.subdivide(points);
        }
    }

    fn calculate(&mut self, edges: &mut [Edge], points: &mut [V3]) {
        let side_length = self.calculate_edges(edges, points) + 1;

        if side_length > 2 {
            let ab = if self.ab_forward {
                Slice::Forward(&edges[self.ab_edge].points)
            } else {
                Slice::Backward(&edges[self.ab_edge].points)
            };
            let bc = if self.bc_forward {
                Slice::Forward(&edges[self.bc_edge].points)
            } else {
                Slice::Backward(&edges[self.bc_edge].points)
            };
            let ca = if self.ca_forward {
                Slice::Forward(&edges[self.ca_edge].points)
            } else {
                Slice::Backward(&edges[self.ca_edge].points)
            };
            self.contents.calculate(ab, bc, ca, points);
        }
    }

    fn add_indices(&self, buffer: &mut Vec<u32>, edges: &[Edge]) {
        let ab = if self.ab_forward {
            Slice::Forward(&edges[self.ab_edge].points)
        } else {
            Slice::Backward(&edges[self.ab_edge].points)
        };
        let bc = if self.bc_forward {
            Slice::Forward(&edges[self.bc_edge].points)
        } else {
            Slice::Backward(&edges[self.bc_edge].points)
        };
        let ca = if self.ca_forward {
            Slice::Forward(&edges[self.ca_edge].points)
        } else {
            Slice::Backward(&edges[self.ca_edge].points)
        };

        add_indices_triangular(self.a, self.b, self.c, ab, bc, ca, &self.contents, buffer);

        self.contents.add_indices(buffer);
    }
}

fn add_indices_triangular(
    a: u32,
    b: u32,
    c: u32,
    ab: Slice<'_>,
    bc: Slice<'_>,
    ca: Slice<'_>,
    contents: &TriangleContents,
    buffer: &mut Vec<u32>,
) {
    let subdivisions = ab.len();
    if subdivisions == 0 {
        buffer.extend_from_slice(&[a, b, c]);
        return;
    } else if subdivisions == 1 {
        buffer.extend_from_slice(&[a, ab.at(0), ca.at(0)]);
        buffer.extend_from_slice(&[b, bc.at(0), ab.at(0)]);
        buffer.extend_from_slice(&[c, ca.at(0), bc.at(0)]);
        buffer.extend_from_slice(&[ab.at(0), bc.at(0), ca.at(0)]);
        return;
    } else if subdivisions == 2 {
        buffer.extend_from_slice(&[a, ab.at(0), ca.at(1)]);
        buffer.extend_from_slice(&[b, bc.at(0), ab.at(1)]);
        buffer.extend_from_slice(&[c, ca.at(0), bc.at(1)]);

        buffer.extend_from_slice(&[ab.at(1), contents.idx_ab(0), ab.at(0)]);
        buffer.extend_from_slice(&[bc.at(1), contents.idx_ab(0), bc.at(0)]);
        buffer.extend_from_slice(&[ca.at(1), contents.idx_ab(0), ca.at(0)]);

        buffer.extend_from_slice(&[ab.at(0), contents.idx_ab(0), ca.at(1)]);
        buffer.extend_from_slice(&[bc.at(0), contents.idx_ab(0), ab.at(1)]);
        buffer.extend_from_slice(&[ca.at(0), contents.idx_ab(0), bc.at(1)]);
        return;
    }

    let last_idx = ab.len() - 1;

    buffer.extend_from_slice(&[a, ab.at(0), ca.at(last_idx)]);
    buffer.extend_from_slice(&[b, bc.at(0), ab.at(last_idx)]);
    buffer.extend_from_slice(&[c, ca.at(0), bc.at(last_idx)]);

    buffer.extend_from_slice(&[ab.at(0), contents.idx_ab(0), ca.at(last_idx)]);
    buffer.extend_from_slice(&[bc.at(0), contents.idx_bc(0), ab.at(last_idx)]);
    buffer.extend_from_slice(&[ca.at(0), contents.idx_ca(0), bc.at(last_idx)]);

    for i in 0..last_idx - 1 {
        buffer.extend_from_slice(&[ab.at(i), ab.at(i + 1), contents.idx_ab(i)]);
        buffer.extend_from_slice(&[ab.at(i + 1), contents.idx_ab(i + 1), contents.idx_ab(i)]);

        buffer.extend_from_slice(&[bc.at(i), bc.at(i + 1), contents.idx_bc(i)]);
        buffer.extend_from_slice(&[bc.at(i + 1), contents.idx_bc(i + 1), contents.idx_bc(i)]);

        buffer.extend_from_slice(&[ca.at(i), ca.at(i + 1), contents.idx_ca(i)]);
        buffer.extend_from_slice(&[ca.at(i + 1), contents.idx_ca(i + 1), contents.idx_ca(i)]);
    }

    buffer.extend_from_slice(&[
        ab.at(last_idx),
        contents.idx_ab(last_idx - 1),
        ab.at(last_idx - 1),
    ]);

    buffer.extend_from_slice(&[
        bc.at(last_idx),
        contents.idx_bc(last_idx - 1),
        bc.at(last_idx - 1),
    ]);

    buffer.extend_from_slice(&[
        ca.at(last_idx),
        contents.idx_ca(last_idx - 1),
        ca.at(last_idx - 1),
    ]);
}

struct IcoSphere {
    points: Vec<V3>,
    triangles: Vec<Triangle>,
    shared_edges: Vec<Edge>,
    subdivisions: usize,
}

impl IcoSphere {
    fn new(subdivisions: usize) -> Self {
        let mut this = Self {
            points: INITIAL_POINTS.to_vec(),
            shared_edges: {
                let mut edges = Vec::new();
                edges.resize_with(EDGES, Edge::default);
                edges
            },
            triangles: TRIANGLES
                .iter()
                .map(|&(a, b, c, ab_edge, bc_edge, ca_edge)| Triangle {
                    a,
                    b,
                    c,
                    ab_edge,
                    bc_edge,
                    ca_edge,
                    ab_forward: false,
                    bc_forward: false,
                    ca_forward: false,
                    contents: TriangleContents::None,
                })
                .collect(),
            subdivisions: 1,
        };

        let mut new_points = this.points.len();

        for edge in &mut *this.shared_edges {
            edge.subdivide_n_times(subdivisions, &mut new_points);
            edge.done = false;
        }

        for triangle in &mut *this.triangles {
            for i in 0..subdivisions {
                triangle.subdivide(&mut new_points, i);
            }
        }

        let diff = new_points - this.points.len();
        this.points.extend(std::iter::repeat_n(ZERO, diff));

        for triangle in &mut *this.triangles {
            triangle.calculate(&mut this.shared_edges, &mut this.points);
        }

        this.subdivisions = subdivisions;

        this
    }

    fn indices_per_main_triangle(&self) -> usize {
        (self.subdivisions + 1) * (self.subdivisions + 1)
    }

    fn get_indices(&self, triangle: usize, buffer: &mut Vec<u32>) {
        self.triangles[triangle].add_indices(buffer, &self.shared_edges);
    }
}

fn uv_of(point: V3) -> [f32; 2] {
    let inclination = point[1].acos();
    let azimuth = point[2].atan2(point[0]);

    let norm_inclination = inclination / std::f32::consts::PI;
    let norm_azimuth = 0.5 - (azimuth / std::f32::consts::TAU);

    [norm_azimuth, norm_inclination]
}

pub fn icosphere(radius: f32, subdivisions: u32) -> Mesh {
    assert!(
        subdivisions < 80,
        "Cannot create an icosphere of {subdivisions} subdivisions due to there being too many vertices being generated: {} (Limited to 65535 vertices or 79 subdivisions)",
        {
            let temp = subdivisions + 1;
            temp * temp * 10 + 2
        }
    );

    let generated = IcoSphere::new(subdivisions as usize);

    let raw_points = &generated.points;

    let positions: Vec<[f32; 3]> = raw_points.iter().map(|&p| vmul(p, radius)).collect();
    let normals: Vec<[f32; 3]> = raw_points.to_vec();
    let uvs: Vec<[f32; 2]> = raw_points.iter().map(|&p| uv_of(p)).collect();

    let mut indices: Vec<u32> = Vec::with_capacity(generated.indices_per_main_triangle() * 20);

    for i in 0..20 {
        generated.get_indices(i, &mut indices);
    }

    Mesh {
        positions,
        normals,
        uvs,
        indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEVY_ICOSPHERE: [(f32, u32, usize, usize, &str); 6] = [
        (
            1.0,
            1,
            42,
            80,
            "58819F3A63386F9C89B08E4C4AB0C1B59F2DD28247F8245B37CF0D58C4F95795",
        ),
        (
            1.0,
            5,
            362,
            720,
            "B0939AF33378E766D20A273B9BFA846FFB4AB978CD48400AFFEFE956AC4BAB9F",
        ),
        (
            1.0,
            64,
            42252,
            84500,
            "B4B37AB464C3A7434DA5DA2C12ED203F08D15BDF11E9B3A4BDE6A56EF1EE33E1",
        ),
        (
            1.02,
            64,
            42252,
            84500,
            "4044EF96A1C5E99E7245033F705B6546E588F3EF28BC2467BAD45CA710ED2815",
        ),
        (
            1.06,
            64,
            42252,
            84500,
            "44212D8611D976DA7CDD74051D4956CD825E7E1EF939A9BF65FBFD66755F18AE",
        ),
        (
            1.14,
            64,
            42252,
            84500,
            "F10159F5A5FBCA9A4E8015D36FFA9CCD6C91199957704DCCB17A9850A41FF218",
        ),
    ];

    fn bevy_bytes(mesh: &Mesh) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            mesh.positions.len() * 12
                + mesh.normals.len() * 12
                + mesh.uvs.len() * 8
                + mesh.indices.len() * 4,
        );
        for position in &mesh.positions {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for normal in &mesh.normals {
            for value in normal {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for uv in &mesh.uvs {
            for value in uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in &mesh.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes
    }

    fn fingerprint(mesh: &Mesh) -> String {
        crate::digest::sha256_hex(&bevy_bytes(mesh)).to_uppercase()
    }

    #[test]
    fn the_icosphere_is_byte_identical_to_bevys() {
        for (radius, subdivisions, vertices, triangles, expected) in BEVY_ICOSPHERE {
            let mesh = icosphere(radius, subdivisions);
            assert_eq!(
                mesh.vertex_count(),
                vertices,
                "半径 {radius} 细分 {subdivisions} 的顶点数"
            );
            assert_eq!(
                mesh.triangle_count(),
                triangles,
                "半径 {radius} 细分 {subdivisions} 的三角形数"
            );
            assert_eq!(
                fingerprint(&mesh),
                expected,
                "半径 {radius} 细分 {subdivisions} 的字节流与 Bevy 不同"
            );
        }
    }

    #[test]
    fn the_counts_follow_the_closed_forms() {
        for subdivisions in [0_u32, 1, 2, 3, 5, 9] {
            let mesh = icosphere(1.0, subdivisions);
            let side = subdivisions + 1;
            assert_eq!(
                mesh.vertex_count() as u32,
                10 * side * side + 2,
                "细分 {subdivisions} 的顶点数"
            );
            assert_eq!(
                mesh.indices.len() as u32,
                60 * side * side,
                "细分 {subdivisions} 的索引数"
            );
        }
    }
}
