//! 球的镶嵌：**逐字节复刻** Bevy 0.19 的 `Sphere::new(r).mesh().ico(s)`。
//!
//! 为什么要"逐字节"而不是"几何等价"：渲染宿主与烘图侧（Bevy）必须生出**同一条**网格。
//! §105 那批锚读数是逐像素哈希，顶点序、索引序、UV 差一位读数就不可比 —— 判据只能是
//! "与 Bevy 落盘的字节流完全相同"。数学上等价的写法（换一种细分顺序、把内点一次算完而不是
//! 覆盖写）会得到同一个球，却得到**不同**的顶点序，哈希当场红。
//!
//! 出处：`bevy_mesh-0.19.1/src/primitives/dim3/sphere.rs:84-162`（`ico()`），它调
//! `hexasphere-18.0.0` 的 `shapes::IcoSphere::new(subdivisions, uv_fn)`。下面是 hexasphere
//! 那条链路的**直译**：`Subdivided` 的构造顺序、`TriangleContents` 的五种形态、
//! `add_indices_triangular` 的分支全部照抄，没有走"把数学重写一遍"的路。
//!
//! ⚠ 三处最容易"顺手改错"的地方（都是上游既有行为，不是笔误）：
//!
//! 1. **分配顺序**：30 条棱先各拿 `s` 个点（下标 `12..12+30s`），20 个主三角形的内点**在之后**
//!    才追加（`lib.rs:1109-1118`）；每个主三角形的内点数是 `s(s-1)/2`（`i == 0` 那次空转，
//!    `lib.rs:950-954`），不是 `s(s+1)/2`。
//! 2. **棱的方向**：`Edge::default()` 是 `done: true`，构造时被重置成 `false`（`lib.rs:1109-1111`）；
//!    第一个碰到某条棱的主三角形**按自己 a→b 的方向**算出这条棱的点并置 `forward = true`，
//!    后来者一律 `forward = false`、反向读（`lib.rs:914-941`）。值和对外的绕向都吃这条。
//! 3. **`More` 覆盖自己的槽位**：`sides` 每层追加一组三个，`More::calculate` 又把
//!    `sides[0..L] / [L..2L] / [2L..3L]` 整段重写成 a→b、b→c、c→a 的内点，随后内层
//!    `contents.calculate` 还会覆盖其中一部分（`lib.rs:514-531`、`555-602`）。顺序反了值就错。
//!
//! 闭式（`sphere.rs:96-115` 的推导，下面 `tests` 里钉住）：`vertices = 10(s+1)² + 2`、
//! `indices = 60(s+1)²`。`s = 64` ⇒ 42252 个顶点、253500 个索引。

use crate::mesh::Mesh;

type V3 = [f32; 3];

const ZERO: V3 = [0.0, 0.0, 0.0];

/// 主三角形的棱数（hexasphere `shapes.rs:1141`）。
const EDGES: usize = 30;

/// 二十面体的 12 个初始顶点 —— **字面量表**，不是算出来的（`shapes.rs:785-842`）。
///
/// ⚠ 表里有几个"同一个数、写法不同"的字面量（顶点 3 的 `-0.72360679774997882507` 与顶点 4 的
/// `-0.72360679774997904712`）：上游就是这么写的，照抄，别顺手统一 —— 它们各自按最近的
/// `f32` 取整，改一位就可能改一个 bit。
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

/// 20 个主三角形：`(a, b, c, ab_edge, bc_edge, ca_edge)`（`shapes.rs:874-1139`）。
///
/// 上游那 20 条里 `ab/bc/ca_forward` 全是 `false`、`contents` 全是 `TriangleContents::None`，
/// 所以这里只留六个真正带信息的字段（那份表是 `const`，`Box::new` 每次调用重新内联一份）。
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

/// glam 的 `Vec3A` 在这里退化成三个 `f32`。
///
/// ⚠ `vdot` 的求和次序必须是 `(x*x + y*y) + z*z`：glam 0.32 的 SSE2 / NEON / coresimd 三条
/// 实现都是这个次序（`src/sse2.rs:42-48`、`src/neon.rs:24-30`、`src/coresimd.rs:5-11`），
/// 换个括号最后一位就变。Rust 不做 FMA 收缩，所以标量写法与 SIMD 逐位一致。
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

/// 半步插值（`interpolation.rs:23-25`）：`(a+b) / sqrt(2(1+a·b))`。
///
/// ⚠ 这**不是** `normalize()`：`2.0 * (1.0 + a.dot(b))` 与 glam 的 `(a+b).length_squared()`
/// 是两次不同的浮点运算，末位不同。icosphere 全程不调用 `normalize()`，所以点只是"差不多"
/// 落在单位球上 —— 照抄这个算式，别图省事换成归一化。
#[inline]
fn geometric_slerp_half(a: V3, b: V3) -> V3 {
    vmul(vadd(a, b), (2.0 * (1.0 + vdot(a, b))).sqrt().recip())
}

/// 把 `indices` 这批点按等分角度插到 `a → b` 的弧上（`interpolation.rs:35-45`）。
///
/// 等分比例 `(i+1)/(n+1)` 必须按 `f32` 一步一步算出来；写成别的等价形式（例如
/// `1.0 - (n-i)/(n+1)`）会在末位分叉。
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

/// 可正可反的一段下标（`slice.rs:8-32`）：`Backward` 只是把下标倒过来读，值不动。
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

/// 两个主三角形共享的一条棱（`lib.rs:246-274`）。
///
/// ⚠ `done` 的默认值是 `true`：它是"这条棱还没算"的**标记**，构造时统一重置成 `false`
/// （`lib.rs:1111`）。谁要是"顺手"把默认值改成 `false`，第二次构造就会被上一次的状态污染。
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

/// 一个主三角形**内部**的点（不含边界，边界由 `Edge` 持有）：`lib.rs:284-327`。
///
/// 五种形态对应细分层数 1/2/3/4/>4。它同时是"槽位表"和"递归结构"：`sides` 里存的是下标，
/// 值由后面的 `calculate` 阶段填 —— 所以同一个下标会先当 A 边的点、再被内层覆盖成别的点。
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

    /// 细分一层：**只分配槽位，不算值**（`lib.rs:495-534`）。值在 `calculate` 阶段统一填。
    ///
    /// `More` 那支的 `*points += 3` 必须排在 `contents.subdivide` **前面**：槽位分配的先后
    /// 就是顶点下标的先后，换一行整批下标就错位。
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

    /// 填值（`lib.rs:536-604`）。边界上的点由调用方以 `Slice` 传进来，方向、长度都由它决定。
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

                geometric_slerp_multiple(points[a_idx as usize], points[b_idx as usize], ab, points);
                geometric_slerp_multiple(points[b_idx as usize], points[c_idx as usize], bc, points);
                geometric_slerp_multiple(points[c_idx as usize], points[a_idx as usize], ca, points);

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

/// 一个主三角形（`lib.rs:839-851`）。`ab/bc/ca_forward` 记录"这条棱的点是不是按我这个方向
/// 算的" —— 见文件头 ⚠ 第 2 条。
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
    /// 算这条三角形的三条棱 —— **只有第一个碰到某条棱的三角形真的算**（`lib.rs:914-941`）。
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

    /// `subdivision_level == 0` 那次是**空转**（`lib.rs:950-954`）：`s = 1` 时一个内点都没有。
    fn subdivide(&mut self, points: &mut usize, subdivision_level: usize) {
        if subdivision_level >= 1 {
            self.contents.subdivide(points);
        }
    }

    /// `side_length > 2` 才碰内点（`lib.rs:956-977`）：`s <= 1` 时 `contents` 还是 `None`，
    /// 而 `None` 的 `calculate` 会 panic —— 这个门槛是必需的，不是优化。
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

/// 把一层三角形按**固定的绕向**吐进索引缓冲（`lib.rs:1504-1580`）。
///
/// ⚠ 两个"看着像手滑、其实不能改"的地方：
/// - `subdivisions == 2` 那支的第二、三行也调 `contents.idx_ab(0)`（`lib.rs:1530-1531`、`1535`）。
///   只因为 `s = 2` 时 `contents` 必然是 `One`，三个 `idx_*` 返回同一个下标才没出事 —— 照抄。
/// - 一般支的 `last_idx - 1` 是特例收尾，`for` 循环故意只跑到 `0..last_idx - 1`（`lib.rs:1549`）。
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

/// 细分出来的一条球（hexasphere 的 `Subdivided<[f32; 2], IcoSphereBase>`，`lib.rs:1055-1132`）。
struct IcoSphere {
    points: Vec<V3>,
    triangles: Vec<Triangle>,
    shared_edges: Vec<Edge>,
    subdivisions: usize,
}

impl IcoSphere {
    /// 构造顺序就是 `new_custom_shape` 的顺序，一步都不能挪（`lib.rs:1089-1132`）：
    ///
    /// 1. 12 个初始点；
    /// 2. 30 条棱各分 `s` 个**下标**（`12..12+30s`），并把 `done` 全部重置成 `false`；
    /// 3. 20 个主三角形按 `i = 0..s` 细分 —— `i == 0` 空转，所以每层加 `i` 个内点；
    /// 4. 补零到最终长度；
    /// 5. 再按主三角形顺序 `calculate`：这一步才真的算坐标（棱先算，内点后算）。
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

    /// ⚠ 名字叫 `indices_per_main_triangle`，返回的却是**每主三角形的三角形个数** `(s+1)²`
    /// （`lib.rs:1380-1382`）。Bevy 拿它当容量 `* 20`（`sphere.rs:146`），只有真正要写的
    /// 三分之一 —— 索引缓冲会再扩一次。这里照抄，因为它不影响内容，只影响"这句话什么意思"。
    fn indices_per_main_triangle(&self) -> usize {
        (self.subdivisions + 1) * (self.subdivisions + 1)
    }

    fn get_indices(&self, triangle: usize, buffer: &mut Vec<u32>) {
        self.triangles[triangle].add_indices(buffer, &self.shared_edges);
    }
}

/// UV：Bevy 传进来的那个闭包（`sphere.rs:121-129`），逐行照抄。
///
/// `acos`/`atan2` 走的是 std：默认构建里 `bevy_math::ops` 选 `std_ops`（`ops.rs:612-613`，
/// `libm` 没开），映射到 `f32::acos` / `f32::atan2`（`ops.rs:134-136`、`156-158`）；
/// hexasphere 的 `math.rs:20-37` 同理。它们就是**平台 libm**，不是正确舍入的 —— 同一台机器
/// 同一条工具链逐位一致，换平台可能差几个 ULP。
///
/// ⚠ 返回顺序是 `[u, v] = [norm_azimuth, norm_inclination]`，别按直觉写成 `[v, u]`；
/// 极点处 `atan2(0, 0) == 0` ⇒ 两个极点的 u 都是 0.5。
fn uv_of(point: V3) -> [f32; 2] {
    let inclination = point[1].acos();
    let azimuth = point[2].atan2(point[0]);

    let norm_inclination = inclination / std::f32::consts::PI;
    let norm_azimuth = 0.5 - (azimuth / std::f32::consts::TAU);

    [norm_azimuth, norm_inclination]
}

/// 二十面体球 —— 与 Bevy 的 `Sphere::new(radius).mesh().ico(subdivisions)` 逐字节相同。
///
/// 三条属性的口径（`sphere.rs:131-144`）：位置 = 归一化点 × `radius`；法线 = 那个点**不缩放**；
/// UV = 上面那个闭包。所以法线**不是**严格的单位向量（slerp 只近似保长），Bevy 也是这样。
///
/// `subdivisions >= 80` 在 Bevy 那边是 `Err(TooManyVertices)`，而 `MeshBuilder::build()`
/// 直接 `unwrap()` —— 这里同样 panic（`10(s+1)² + 2` 会越过 65535 顶点）。
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

    /// 期望值不是"跑一遍自己算出来的"——那样只能证明自己跟自己一致。它们是真实 Bevy 落盘的
    /// `target/oracle/bevy-icosphere-*.bin` 的 SHA-256（那批文件是逐字节的网格流，见下）。
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
        // ⚠ 这一格不是凑数：`art/scene/orbit-bare*.toml` 的 atmosphere 件 `outer = 1.14`，
        // 落进产物就是 `{"name":"icosphere","params":{"radius":1.1399999856948853,
        // "subdivisions":64.0}}` —— **判据场景真正用的那颗球**。
        (
            1.14,
            64,
            42252,
            84500,
            "F10159F5A5FBCA9A4E8015D36FFA9CCD6C91199957704DCCB17A9850A41FF218",
        ),
    ];

    /// oracle 的字节口径：`positions → normals → uvs → indices`，全小端，无表头、无填充。
    /// 顺序、端序、有没有表头都是**判据的一部分** —— 换一格，五个哈希全部作废。
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

    /// 顶点序 / 索引序 / UV / 半径缩放，只要有一处与 Bevy 不同，这里的哈希就红。
    /// 半径只乘在位置上（法线不缩放），所以三条 `s = 64` 的哈希在前半段相同、后半段不同 ——
    /// 三个一起钉，等于同时验了"半径只影响位置"这条口径。
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

    /// 闭式（`sphere.rs:96-115`）：`vertices = 10(s+1)² + 2`、`indices = 60(s+1)²`。
    ///
    /// 它和哈希是两种判据：哈希管"是不是同一条网格"，闭式管"数量级有没有崩"（例如某个
    /// `subdivide` 分支被改了层数，哈希会红但闭式能立刻说明是数量错了）。
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
