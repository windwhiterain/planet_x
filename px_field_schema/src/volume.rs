//! **体网格的读法**（`Domain::Volume`）：行号 → `(面, 层, 面内 s, t)`，以及这一格在世界里的点。
//!
//! 体网格是一张普通场（`width × height` 的 f32），多出来的那一维折进了 `height`：
//!
//! ```text
//! width  = res                          （一面之内，s 方向的采样点数）
//! height = res × res × layers × 6       （一面 = 一块 res × (layers × res) 的平面）
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

use px_protocol::art::{CUBE_FACES, Domain};

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

/// 一行 → 它的 **3D 体素坐标**：面内 `(s, t)` 与归一化径向高度 `altitude`，都在 `[0,1]`。
///
/// 每面再加一个只由面号决定的固定偏移（见 [`face_offset`]）—— 于是六面各自占噪声空间里
/// 一块**互不重叠**的区域，而接缝处仍然连续（相邻面在公共棱上取到的是同一条棱上的点，
/// 只是各自带了不同的偏移 ⇒ ⚠ **这一档在接缝上并不逐位相等**；体网格是给体渲染用的，
/// 它按**方向**采样（`VolumeGrid` 那种读法），不靠"面与面逐位对齐"来焊合。
///
/// ⚠ `altitude` 归一化到 `[0,1]`（与 `s` / `t` 同量纲）⇒ 噪声空间是一个**立方体**。
///   不归一化（径向用 `0..layers`）的话噪声格在径向上被拉扁，结构会沿径向拉长。
///
/// ⚠ **要"面内坐标"就用 [`local_voxel_of`]，不要把这一份减掉偏移**：偏移的量级可以到
///   `31`（见 [`face_offset`]），在 `f32` 上"加上去再减回来"会丢掉低位 —— 实测 `res = 8`
///   时那样反解出来的格心偏差达 `0.55`（整个值域的一半）。权威版本是**根本不加**。
pub fn voxel_of(shape: &VolumeShape, face: u32, x: u32, y: u32) -> [f32; 3] {
    let local = local_voxel_of(shape, face, x, y);
    let offset = face_offset(face);
    [
        local[0] + offset[0],
        local[1] + offset[1],
        local[2] + offset[2],
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

/// 面号 → 噪声空间里的固定偏移。
///
/// ⚠ **必须是"非整数且互不成比例"的数**（这一条是实测出来的）：整数偏移会让六面的
///   `(s, t, altitude)` 落在**同样的格内位置**上（小数部分一样），于是六面取到的是
///   同一个噪声函数、只差一个整数平移 —— 出来的就是**六块彼此旋转复制的云**
///   （接缝照样连续，所以看上去不是裂缝而是"对称的花纹"，很难归因）。
///   小数部分错开之后六面才是各自的噪声区域。
///
/// ⚠ 三个分量也不许成比例（比如都用 `7.31`）：那样六个点在噪声空间里排成一条直线，
///   图案会带上一个方向性。
///
/// ⚠ **它是公开的**：读体素坐标的人（`field.warp3` 的三线性采样）必须把这一份减掉才能
///   落回"面内位置" —— 网格的行列只按面内位置排。这是"布局只有一处真源"的一部分。
pub fn face_offset(face: u32) -> [f32; 3] {
    const UNIT: [[f32; 3]; CUBE_FACES as usize] = [
        [0.0, 0.0, 0.0],
        [7.31, 3.17, 11.93],
        [5.47, 19.63, 2.71],
        [22.19, 4.88, 17.03],
        [9.11, 27.37, 6.23],
        [31.07, 12.53, 23.89],
    ];
    UNIT[(face % CUBE_FACES) as usize]
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
        assert_eq!(shape.height(), 4 * 4 * 3 * CUBE_FACES);
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

    /// **体素坐标落在 `[0,1]` 附近、径向按层单调**：它是噪声的采样点，量纲必须一致。
    #[test]
    fn a_voxel_coordinate_is_normalised_and_climbs_with_the_layer() {
        let shape = shape();
        let mut last = f32::NEG_INFINITY;
        for layer in 0..shape.layers {
            let row = shape.row_of(0, layer);
            let voxel = voxel_of(&shape, 0, 1, row);
            assert!((0.0..=1.0).contains(&voxel[2]), "高度跑出 [0,1]：{voxel:?}");
            assert!(voxel[2] > last, "层号越大高度必须越大：{voxel:?}");
            last = voxel[2];
        }
        // 面内两维是格心 ⇒ 落在 (0, 1) 开区间里。
        let voxel = voxel_of(&shape, 0, 2, 3);
        assert!(voxel[0] > 0.0 && voxel[0] < 1.0, "{voxel:?}");
        assert!(voxel[1] > 0.0 && voxel[1] < 1.0, "{voxel:?}");
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

    /// **六面在噪声空间里不重叠**：同一个 `(s, t, altitude)` 在六面上必须给出六个不同的点。
    ///
    /// ⚠ 这条判的就是"六面会不会长成同一个样子" —— 重叠时接缝仍然连续，
    ///   所以画面上是**对称花纹**而不是裂缝，靠眼睛很难归因到布局。
    #[test]
    fn the_six_faces_do_not_share_one_noise_region() {
        let shape = shape();
        let mut points: Vec<[f32; 3]> = Vec::new();
        for face in 0..CUBE_FACES {
            points.push(voxel_of(&shape, face, 1, 1));
        }
        for one in 0..points.len() {
            for two in (one + 1)..points.len() {
                let gap = (0..3)
                    .map(|axis| (points[one][axis] - points[two][axis]).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    gap > 1.0,
                    "面 {one} 与面 {two} 的采样点几乎重合（差 {gap}）"
                );
            }
        }
    }
}
