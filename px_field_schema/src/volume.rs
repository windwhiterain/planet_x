//! **体网格的读法**（`Domain::Volume`）：行号 → `(面, 层, 面内 s, t)`，以及这一格在世界里的点。
//!
//! 体网格是一张普通场（`width × height` 的 f32），多出来的那一维折进了 `height`：
//!
//! ```text
//! width  = res                          （一面之内，s 方向的采样点数）
//! height = res × layers × 6             （一面 = 一块 res × (layers × res) 的平面）
//! data   = [face][layer][t][s]          （与 `VolumeData` 的摊平顺序**逐字相同**）
//!
//! 行号   = face × (res × layers) + layer × res + t      ← t 是**层内**的纵向格号
//! ```
//!
//! ⚠⚠ **`t` 是层内的纵向格号，`layer × res + t` 才是面内行号**（一"层"占 `res` 行，
//!   一层之内行是连续的）。这两个量**不能混**：把 `layer` 和 `t` 挤进同一维
//!   （`y = face × layers + layer`）会让一面只剩 `layers` 行，而一面需要 `res × layers` 行
//!   —— 那正是这份注释第一版写错的地方，也是 `field.warp3` 里"格心采样偏 0.40"的根。
//!
//! ⚠ **为什么布局是这样**（三条约束，只有这一个解）：
//!   1. 必须与 `px_protocol::art::VolumeData` 的 `[面, 层, t, s]` 一致 —— 否则
//!      "场 ↔ 体积"的搬运要逐元素重排（那是一次沉默的错位来源）；
//!   2. 一层之内的行连续（`layer × res + t`）⇒ 按 `(层, t)` 回读网格只要整数运算；
//!   3. `sample`（体渲染的三线性读法）天然按这个顺序写。
//!
//! ⚠ **世界点映射的必要条件**：`px_protocol::art::direction_at` / `uv_of` 是**球面**的入口
//!   （它们只认得出"一个方向"），而体网格的每一格多一维径向层 ⇒ 那一侧拿不到真正的点
//!   （它的 `height` 里混着面号与层号，解不出 `layers`）。所以世界点映射**只在这一份里**
//!   —— 这是那件事的唯一真源，别处再写一遍就是第二个会漂开的真相。

use px_protocol::art::{CUBE_FACES, Domain, cube_direction};

use crate::field::Field;
use px_graph_schema::Grid;

/// 一份体网格的**形状**：面内分辨率与径向层数。
///
/// ⚠ **它完全由画布推出来**（`width = res`、`layers = height / (res × 6)`）⇒
///   体网格**不需要**任何额外的载荷、清单参数或新资产种类 —— 一张 `Field` 加一个
///   域就是全部。这正是"把第三维折进 `height`"买到的东西。
///
/// ⚠ 半径（`inner` / `outer`）**不在这里**：它不是网格的形状，是"这张网格摆在世界的哪"。
///   只有要**世界点**的算子才需要它（体渲染的视线求交），那份算子自己带这个参数。
///   逐格算子（噪声、重映射、混合、扭曲）只在 `[0,1]³` 的体素坐标里干活，用不着半径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeShape {
    pub res: u32,
    pub layers: u32,
}

impl VolumeShape {
    /// 从画布推形状。`None` = 这张画布不是体网格（域不对，或者行数除不出整数层）。
    pub fn of(grid: &Grid) -> Option<Self> {
        if grid.projection != Domain::Volume {
            return None;
        }
        let res = grid.width.max(1);
        let layers = px_protocol::art::volume_layers(grid.height, res)?;
        Some(Self { res, layers })
    }

    /// 这个形状对应多少行。⚠ 它就是画布的 `height`（一面 = `res × (layers × res)`）。
    pub fn height(&self) -> u32 {
        px_protocol::art::volume_extent(self.res, self.layers).1
    }

    /// 行号 → `(面, 层)`。
    ///
    /// 行号是 `face × (res × layers) + layer × res + t` ⇒ 先整除去掉面与层，剩下的就是 `t`。
    /// ⚠ 这里**必须**用 `res × layers`（一面占多少行），不是 `layers`：一面是一个
    ///   `res × (layers × res)` 的平面（见本模块的文件头）。
    pub fn slot_of(&self, y: u32) -> Option<(u32, u32)> {
        let res = self.res.max(1);
        let layers = self.layers.max(1);
        if y >= self.height() {
            return None;
        }
        let plane = res * layers;
        let face = y / plane;
        let layer = (y % plane) / res;
        Some((face.min(CUBE_FACES - 1), layer.min(layers - 1)))
    }

    /// `(面, 层)` → 那**一层的第一行**（[`Self::slot_of`] 的逆，`t = 0`）。
    pub fn row_of(&self, face: u32, layer: u32) -> u32 {
        let res = self.res.max(1);
        let layers = self.layers.max(1);
        face * (res * layers) + layer.min(layers - 1) * res
    }

    /// 这张场是不是这个形状的体网格（域对 + 行列数对）。
    ///
    /// ⚠ 判据**只有这一处**：算子、判据仪器、搬运都问它，免得各写一份"行数 = res × layers × 6"。
    pub fn matches(&self, field: &Field) -> bool {
        field.projection == Domain::Volume
            && field.width == self.res.max(1)
            && field.height == self.height()
    }
}

/// 一行 → 它的**三维采样点**（方向 × 半径）—— 噪声/密度是**位置的三维场**。
///
/// ⚠⚠ **面只是"方向的参数化工具"**（round 29 改）：采样点取
///   `cube_direction(face, s, t) × 半径`，而 `cube_direction` 在公共棱上给出**同一个方向**
///   ⇒ 同一个三维点无论从哪一面取样都是**同一个值** ⇒ **棱上没有缝**。
///
/// ⚠ 六面天然各占三维空间里不同的一块（方向不同）⇒ **不再需要"每面一个固定偏移"**。
///   从前那份偏移让相邻两面在公共棱上取到**不同的值** —— 场本身就是断的，接缝是它的
///   必然后果（当时它的理由是"防六面长得一样"，而那条需求由"按三维位置取噪声"天然满足）。
///
/// ⚠ 半径 = [`SHELL_FRONT`] + `altitude`：`FRONT = 2/π` 让一面的弧长
///   `(π/2)·FRONT = 1.0` 与径向跨度 `1.0` **同量纲** ⇒ 噪声各向同性；旧坐标也是
///   "一面一单位、径向一单位"，所以频率参数不用重调。
///
/// ⚠ **要"面内坐标"就用 [`local_voxel_of`]**（读网格的行列用它）—— 这一份是噪声空间。
pub const SHELL_FRONT: f32 = std::f32::consts::FRAC_2_PI;

pub fn voxel_of(shape: &VolumeShape, face: u32, x: u32, y: u32) -> [f32; 3] {
    let local = local_voxel_of(shape, face, x, y);
    let radius = SHELL_FRONT + local[2];
    let direction = cube_direction(face % CUBE_FACES, local[0], local[1]);
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

/// 行号 → **这一面之内的行号**（`0..res × layers`），即 `layer × res + t`。
///
/// ⚠ 它是**从第 0 面起数**的口径（`y % (res·layers)`）。要"某一面的面内行号"就减去那一面的
///   起点（见 [`local_voxel_of`]）—— 面是权威，行号单独一个数分不出自己在哪一面。
pub fn in_face_row(shape: &VolumeShape, y: u32) -> u32 {
    let res = shape.res.max(1);
    y % (res * shape.layers.max(1))
}

/// 行号 → 面内**层内的纵向格号** `t`（`0..res`）。
pub fn row_within_layer(shape: &VolumeShape, y: u32) -> u32 {
    in_face_row(shape, y) % shape.res.max(1)
}

/// 行号 → **面内行号** → 层号（整数运算，不经过归一化高度）。
///
/// ⚠ 这是"按坐标回读网格"唯一正确的路：整数除法不会因舍入跳层（见 [`local_voxel_of`]）。
pub fn layer_of(shape: &VolumeShape, y: u32) -> u32 {
    in_face_row(shape, y) / shape.res.max(1)
}

/// 行号 → **面内坐标** `(s, t, altitude)`（`t` 是**层内**纵向格号，**不含**面偏移）。
///
/// ⚠ **这一份是"网格的行列"那一套坐标**（读网格用它），而 [`voxel_of`] 是"噪声空间"
///   那一套（取噪声用它）。两者的差别**只有面偏移**。
///
/// ⚠⚠ **面内行号要按这一面的起点算**（`face` 是权威）：只拿 `y` 去 `% (res·layers)` 是
///   "从第 0 面起数"，六面会全部落到同一块的同一个位置上 —— 实测 `cloud.density` 把第 5
///   面的值读成第 0 面的，偏差 6000（整整六个面的行距）。传进来的 `face` 与 `y` 说的是
///   同一格，用 `face` 的起点就没有这个歧义。
///
/// ⚠ 层号必须走**整数除法**，不能用 `altitude × (layers-1)` 反解：`f32` 上
///   `1/3 × 3 = 1.0000001` ⇒ `floor` 跳一层（实测偏差 0.40，近半个值域）。
pub fn local_voxel_of(shape: &VolumeShape, face: u32, x: u32, y: u32) -> [f32; 3] {
    let res = shape.res.max(1);
    let last = shape.layers.max(2) - 1;
    let in_face = y.saturating_sub(face * res * shape.layers.max(1));
    let layer = in_face / res;
    let t = in_face % res;
    [
        (x as f32 + 0.5) / res as f32,
        (t as f32 + 0.5) / res as f32,
        layer.min(last) as f32 / last as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> VolumeShape {
        VolumeShape { res: 4, layers: 3 }
    }

    /// **行 ↔ `(面, 层)` 互逆**，而且面与层都落在合法范围里。
    ///
    /// ⚠ 这是整条体网格路径的底层约定：错了的话"这一格在世界里的哪"会整片错位，
    ///   而画面上只表现为"形状不对"，归因不到布局。
    #[test]
    fn a_row_decodes_back_to_its_own_face_and_layer() {
        let shape = shape();
        assert_eq!(shape.height(), 4 * 3 * CUBE_FACES);
        let mut seen = 0;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                let row = shape.row_of(face, layer);
                assert_eq!(shape.slot_of(row), Some((face, layer)));
                seen += 1;
            }
        }
        assert_eq!(seen, CUBE_FACES * shape.layers);
        assert_eq!(
            shape.slot_of(shape.height()),
            None,
            "越界的行要明确说不认识"
        );
    }

    /// **形状完全由画布推出来**（这是"不需要新资产种类"那条承诺的判据）。
    #[test]
    fn the_shape_comes_back_out_of_the_canvas() {
        let shape = shape();
        let grid = Grid {
            width: shape.res,
            height: shape.height(),
            projection: Domain::Volume,
        };
        assert_eq!(VolumeShape::of(&grid), Some(shape));
        // 行数除不出整数层 ⇒ 不是体网格。
        let ragged = Grid {
            height: shape.height() + 1,
            ..grid
        };
        assert_eq!(VolumeShape::of(&ragged), None);
        // 域不对 ⇒ 不是体网格（哪怕行列数碰巧对得上）。
        let flat = Grid {
            projection: Domain::CubeMap,
            ..grid
        };
        assert_eq!(VolumeShape::of(&flat), None);
    }

    /// **`matches` 认的是域 + 两个尺寸**：域对、行列数对才算这个形状的体网格。
    #[test]
    fn a_volume_needs_the_domain_and_both_extents() {
        let shape = shape();
        let good = Field::filled_with(shape.res, shape.height(), 0.0, Domain::Volume);
        assert!(shape.matches(&good));
        let wrong_domain = Field::filled_with(shape.res, shape.height(), 0.0, Domain::CubeMap);
        assert!(!shape.matches(&wrong_domain));
        let wrong_rows = Field::filled_with(shape.res, shape.height() - 1, 0.0, Domain::Volume);
        assert!(!shape.matches(&wrong_rows));
        let wrong_columns = Field::filled_with(shape.res + 1, shape.height(), 0.0, Domain::Volume);
        assert!(!shape.matches(&wrong_columns));
    }

    /// **体素坐标是"方向 × 半径"**：半径随层单调、落在这层壳里。
    #[test]
    fn a_voxel_coordinate_is_a_point_in_three_dimensional_space() {
        let shape = shape();
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut last = f32::NEG_INFINITY;
        for layer in 0..shape.layers {
            let row = shape.row_of(0, layer);
            let radius = length(voxel_of(&shape, 0, 1, row));
            assert!(
                (SHELL_FRONT..=SHELL_FRONT + 1.0 + 1e-4).contains(&radius),
                "半径跑出壳：{radius}"
            );
            assert!(radius > last, "层号越大半径必须越大：{radius}");
            last = radius;
        }
    }

    /// ⚠⚠ **接缝的判据**：相邻两面在公共棱上取到的是**同一个三维点**。
    ///
    /// 面 0（`+x`）的 `s = 0` 棱与面 4（`+z`）的 `s = 1` 棱是同一条线 —— `cube_direction`
    /// 在面 0 给 `[1, -b, -a]`（`a = -1` ⇒ `[1, -b, 1]`）、在面 4 给 `[a, -b, 1]`
    /// （`a = 1` ⇒ 同一个）。两边**格心**各离棱半个纹素 ⇒ 距离只有一两个纹素；
    /// 从前那份"每面一个固定偏移"会让它差到 `2.3` 以上 —— 那就是画面上那道缝的根。
    #[test]
    fn the_two_faces_meet_on_the_shared_edge() {
        let shape = VolumeShape { res: 8, layers: 4 };
        let texel = std::f32::consts::FRAC_PI_2 * SHELL_FRONT / shape.res as f32;
        for layer in 0..shape.layers {
            let left = voxel_of(&shape, 0, 0, shape.row_of(0, layer) + 3);
            let right = voxel_of(&shape, 4, shape.res - 1, shape.row_of(4, layer) + 3);
            let gap = (0..3)
                .map(|axis| (left[axis] - right[axis]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(
                gap < texel * 3.0,
                "公共棱两侧的点差了 {gap}（一个纹素约 {texel}）—— 场在棱上是断的"
            );
        }
    }

    /// **面内坐标随行号走**：`t` 循环 `0..res`、层号递增，跨面时重新从 `(0, 0)` 起。
    ///
    /// ⚠ 这条钉的是"面号必须被剥掉"：不剥的话第 1 面会被读成第 0 面（实测：
    ///   `cloud.density` 把第 1 面的值全部搬到第 0 面上，而画面上只表现为"六面一样"）。
    #[test]
    fn the_in_face_coordinates_do_not_leak_across_faces() {
        let shape = VolumeShape { res: 4, layers: 3 };
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                for t in 0..shape.res {
                    let y = shape.row_of(face, layer) + t;
                    assert_eq!(layer_of(&shape, y), layer, "面 {face} 行 {y} 的层号");
                    assert_eq!(
                        row_within_layer(&shape, y),
                        t,
                        "面 {face} 行 {y} 的层内格号"
                    );
                    assert_eq!(
                        in_face_row(&shape, y),
                        layer * shape.res + t,
                        "面 {face} 行 {y} 的面内行号"
                    );
                    let voxel = local_voxel_of(&shape, face, 1, y);
                    assert!(
                        (voxel[2] - layer as f32 / (shape.layers - 1) as f32).abs() < 1e-6,
                        "面 {face} 行 {y} 的高度应当是第 {layer} 层"
                    );
                }
            }
        }
    }

    /// **六面在三维空间里不重叠**：同一个 `(s, t, altitude)` 在六面上必须给出六个不同的点。
    ///
    /// ⚠ 这条判的就是"六面会不会长成同一个样子" —— 重叠时（旧坐标里靠"每面一个固定偏移"
    ///   才避开）画面上是**对称花纹**而不是裂缝，靠眼睛很难归因到布局。
    ///   ⚠ 门槛 `0.3` 而不是"差一个整数"：现在是**按位置**取噪声，六个方向本来就只差
    ///   一个面转角（相邻面之间最近），要的是"不是同一个点"，不是"隔多远"。
    #[test]
    fn the_six_faces_do_not_share_one_noise_region() {
        let shape = shape();
        // ⚠ 行号要落在**各自面**的区间里：`local_voxel_of` 用 `saturating_sub` 剥面，
        //   给所有面都传 `y = 1` 会让第 1 面起全部塌到第 0 面的行上（本条第一版就这么错）。
        let mut points: Vec<[f32; 3]> = Vec::new();
        for face in 0..CUBE_FACES {
            points.push(voxel_of(&shape, face, 1, shape.row_of(face, 1) + 1));
        }
        for one in 0..points.len() {
            for two in (one + 1)..points.len() {
                let gap = (0..3)
                    .map(|axis| (points[one][axis] - points[two][axis]).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    gap > 0.3,
                    "面 {one} 与面 {two} 的采样点几乎重合（差 {gap}）"
                );
            }
        }
    }
}
