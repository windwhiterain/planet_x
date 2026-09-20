//! 回读到的字节 → 报告（`px_protocol::render::ShotReport`）里那几个读数。
//!
//! **为什么这一栏要住在宿主里**：读数必须从**回读出来的那些字节**算，不能从磁盘上的 PNG 算
//! —— PNG 是压缩后的东西，而判据看的是原始像素（`px_render/src/main.rs::collect_shot_stat`
//! 顶上那句注释，2026-09-16 那一版）。同理它也不该由一个"再写一个 PNG 解码器"的脚本来算：
//! 那是判据链上一个新的、会自己出错的环节（§122）。
//!
//! ⚠ **口径逐字来自 Bevy 宿主**（`px_render/src/main.rs`）：`GRID_COLS/GRID_ROWS`（:131-132）、
//! `BRIGHT_SUM`（:134）、`CLOUD_DIFF_RATIO`（:140）与 `collect_shot_stat`（:472-505）、
//! `has_cloud`/`verdict` 那一段（:2514-2539）。那几个常数住在**另一个 crate 的 bin** 里，
//! 拿不过来（`px_render` 是个二进制，不对外导出），所以这里是**搬一份**而不是"另定一套"：
//! 口径换一格，`tools/frame-probe.ps1` 打出来的 `has_cloud`/`verdict` 就与历史不可比。
//! ⚠ `px_protocol::render::ShotReport` 的文档注释里写的是"`diff_vs_ref_grid ≥ 0.5% × 总像素`"
//! —— **那句话与代码不一致**（代码是 `1% × 参考图的总光通量`）。这里跟**代码**走：
//! 判据要复现的是"Bevy 宿主实际干了什么"，不是"注释里怎么写的"。

/// 差分网格的列数。参考图与每一张图都按这张网格取"光通量和"。
pub const GRID_COLS: u32 = 16;
/// 差分网格的行数。`16×10` 是 `GRID_COLS` 与它的乘积（160 格）—— 两处一起才是这张网格。
pub const GRID_ROWS: u32 = 10;
/// 亮像素的阈值：`r+g+b > 72/765`。
pub const BRIGHT_SUM: u32 = 72;
/// 云判据的差分阈值比例：网格差分 ÷ **参考图的总光通量**。
pub const CLOUD_DIFF_RATIO: f64 = 0.01;

/// 一张图的三个读数（Bevy 那边叫 `ShotStat`，同一件事）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotStat {
    /// 纯洋红（`r>250 && g<8 && b>250`）像素数 —— 占位 shader 的指纹。
    pub placeholder_px: u64,
    /// 亮像素（`r+g+b > 72`）数。
    pub bright_px: u64,
    /// `16×10` 网格的逐格光通量和（`r+g+b` 的和），长度恒为 `GRID_COLS × GRID_ROWS`。
    pub grid: Vec<u64>,
}

/// 从**回读的 RGBA8** 算一张图的三个读数。
///
/// ⚠ Bevy 那边喂进来的是 `DynamicImage::to_rgb8()`（丢掉 alpha 的 RGB），这里喂的是回读的
/// RGBA —— 三个通道的**值**一模一样（`Rgba8UnormSrgb` 那张图与它的 `to_rgb8()` 同源），
/// 所以三个读数逐位相同。alpha 不参与任何一个读数（`collect_shot_stat` 里就没读它）。
///
/// ⚠ 网格落点那条算式照抄：`(y * GRID_ROWS / height) * GRID_COLS + (x * GRID_COLS / width)`。
/// 两处除法都是**整数**除法，且 `height` 只除行、`width` 只除列 —— 别"理顺"成
/// `y * GRID_ROWS / height` 之外的样子：格子边界落错一格，差分就对不上参考实现。
pub fn measure(pixels: &[u8], width: u32, height: u32) -> ShotStat {
    let mut placeholder_px = 0_u64;
    let mut bright_px = 0_u64;
    let mut grid = vec![0_u64; (GRID_COLS * GRID_ROWS) as usize];
    let width = width.max(1);
    let height = height.max(1);
    for (index, texel) in pixels.chunks_exact(4).enumerate() {
        let (r, g, b) = (texel[0], texel[1], texel[2]);
        if r > 250 && g < 8 && b > 250 {
            placeholder_px += 1;
        }
        let sum = u32::from(r) + u32::from(g) + u32::from(b);
        if sum > BRIGHT_SUM {
            bright_px += 1;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        let cell = (y * GRID_ROWS / height) * GRID_COLS + (x * GRID_COLS / width);
        if let Some(slot) = grid.get_mut(cell as usize) {
            *slot += u64::from(sum);
        }
    }
    ShotStat {
        placeholder_px,
        bright_px,
        grid,
    }
}

/// 两张图的网格差分：逐格光通量之差的绝对值之和。参考图对自己是 0。
pub fn grid_diff(one: &[u64], two: &[u64]) -> u64 {
    one.iter()
        .zip(two.iter())
        .map(|(left, right)| left.abs_diff(*right))
        .sum()
}

/// 差分阈值：按**参考图的总光通量**按比例给（差分与阈值同量纲），下界 1。
///
/// ⚠ 下界 1 是必要的：参考图全黑时阈值为 0 ⇒ "差分 ≥ 0"恒真 ⇒ `has_cloud` 恒真，
/// 那一条判据就变成了永远绿灯（Bevy 那边同一个写法，`.max(1)`）。
pub fn cloud_threshold(reference_luma: u64) -> u64 {
    ((CLOUD_DIFF_RATIO * reference_luma as f64) as u64).max(1)
}

/// 一张图的"可用性"判据的全部输入。打包成一个结构体，是为了让
/// [`cloud_verdict`] 的每一条分支都能被单测**单独**钉住（不是靠一次真渲染碰运气）。
///
/// ⚠ 这里**没有**文档路径那一栏：五条措辞一条都不提它是哪一份产物（Bevy 那边同样不提）。
/// 加一栏没人读的字段，下一次就会有人以为它在参与判据。
pub struct Verdict {
    /// 产物声明了 `clouds` 这个期望标签（`SceneSpec::expects`）。
    pub declared: bool,
    pub placeholder_px: u64,
    pub bright_px: u64,
    /// 与参考图的网格差分（参考图自己是 0）。
    pub diff: u64,
    pub threshold: u64,
    /// 这一张是不是**这一批的第一张**（参考图）。参考图没人可比，判据降一档。
    pub is_reference: bool,
}

/// 可用性判据（`has_cloud` + `verdict` 两栏）。逐条照 `px_render/src/main.rs:2516-2539`。
///
/// ⚠ 分支次序就是语义，尤其这一条：**`placeholder_px > 0` 先判**。装的是占位 shader 时
/// 画面是整块洋红 —— 那张图什么都不能说明，所以它的结论必须是"这一张不算数"，
/// 而不是"该有云却没画出来"（后者会把人引去查云壳，而真因是 shader 没装上）。
pub fn cloud_verdict(state: &Verdict) -> (bool, String) {
    let has_cloud = state.declared
        && state.placeholder_px == 0
        && if state.is_reference {
            state.bright_px > 0
        } else {
            state.diff >= state.threshold
        };
    let verdict = if state.placeholder_px > 0 {
        format!(
            "⚠ 画的是占位 shader：{} 个洋红像素（这一张不算数）",
            state.placeholder_px
        )
    } else if !state.declared {
        "无云档（产物没声明 clouds part）".to_string()
    } else if state.is_reference {
        format!(
            "有云？（这一张是参考图，只能判「不是占位、亮像素 {} > 0」；要判丢云壳得把无云档放这一批第一个）",
            state.bright_px
        )
    } else if has_cloud {
        format!(
            "有云 ✓（与参考图的网格差分 {} ≥ {}）",
            state.diff, state.threshold
        )
    } else {
        format!(
            "⚠ 该有云却没画出来（网格差分 {} < {}）",
            state.diff, state.threshold
        )
    };
    (has_cloud, verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一张 `width×height` 的图：全黑，再把几个像素点成指定的值。
    fn canvas(width: u32, height: u32) -> Vec<u8> {
        vec![0_u8; (width * height * 4) as usize]
    }

    fn put(pixels: &mut [u8], width: u32, x: u32, y: u32, rgb: [u8; 3]) {
        let at = ((y * width + x) * 4) as usize;
        pixels[at] = rgb[0];
        pixels[at + 1] = rgb[1];
        pixels[at + 2] = rgb[2];
        pixels[at + 3] = 255;
    }

    /// 三个读数各自的**边界**都要被点到：
    /// - 洋红（占位）那一档的阈值是 `r>250 && g<8 && b>250` —— 恰好不满足的那一格也要试；
    /// - 亮像素的阈值是 `sum > 72` —— 72 不算、73 才算；
    /// - 网格落点用的是整数除法。⚠ 这一张恰好是 **16×10 = 网格本身的形状** ⇒ 一个像素一格，
    ///   所以下面每一格都能**逐个**断言（16×10 之外的尺寸见下一个测试）。
    #[test]
    fn the_three_readings_have_their_edges_pinned() {
        let (width, height) = (16_u32, 10_u32);
        let mut pixels = canvas(width, height);
        // (0,0) 洋红 ⇒ 占位 +1；它的 r+g+b = 513 > 72 ⇒ 亮像素也 +1（Bevy 那边同样两栏都计）。
        put(&mut pixels, width, 0, 0, [255, 3, 255]);
        // g 恰好是 8 ⇒ **不算**占位（阈值是 `g < 8`）。
        put(&mut pixels, width, 1, 0, [255, 8, 255]);
        // r+g+b = 72 ⇒ **不算**亮（阈值是 `> 72`）。
        put(&mut pixels, width, 2, 0, [24, 24, 24]);
        // r+g+b = 73 ⇒ 算亮，而且**不是**洋红（g 远大于 8）。
        put(&mut pixels, width, 3, 0, [24, 24, 25]);
        // 最后一列最后一行：格子 (15,9) = 下标 9*16+15 = 159。
        put(&mut pixels, width, 15, 9, [10, 10, 10]);
        let stat = measure(&pixels, width, height);
        assert_eq!(stat.placeholder_px, 1, "只有 (0,0) 是洋红");
        assert_eq!(stat.bright_px, 3, "(0,0) 513、(1,0) 518、(3,0) 73 这三格");
        assert_eq!(stat.grid.len(), 160);
        assert_eq!(stat.grid[0], 513);
        assert_eq!(stat.grid[1], 518);
        assert_eq!(stat.grid[2], 72);
        assert_eq!(stat.grid[3], 73);
        assert_eq!(stat.grid[159], 30, "(15,9) 那一格");
        assert_eq!(stat.grid.iter().sum::<u64>(), 513 + 518 + 72 + 73 + 30);
    }

    /// 网格是 `16×10`，而**行/列的除数分别是 height/width** —— 尺寸不是 `16:10` 时最容易
    /// 写反。960×640 的产物就是 3:2（行除 640、列除 960），这一格钉住它；
    /// 顺带钉住"同一格里的多个像素是**累加**"。
    #[test]
    fn the_grid_uses_height_for_rows_and_width_for_columns() {
        let (width, height) = (960_u32, 640_u32);
        let mut pixels = canvas(width, height);
        // 第 319 行（上半的最后一行）与第 320 行（下半的第一行）：同一个格子里各放两个像素。
        put(&mut pixels, width, 0, 319, [10, 0, 0]);
        put(&mut pixels, width, 1, 319, [5, 0, 0]);
        put(&mut pixels, width, 0, 320, [10, 0, 0]);
        // 第 479 列（左半的最后一列）与第 480 列。
        put(&mut pixels, width, 479, 0, [10, 0, 0]);
        put(&mut pixels, width, 480, 0, [10, 0, 0]);
        let stat = measure(&pixels, width, height);
        // y=319: 319*10/640 = 4；y=320: 320*10/640 = 5 ⇒ 行 4 与行 5。
        assert_eq!(stat.grid[4 * 16], 15, "同一格里的两个像素要累加");
        assert_eq!(stat.grid[5 * 16], 10);
        // x=479: 479*16/960 = 7；x=480: 480*16/960 = 8 ⇒ 列 7 与列 8。
        assert_eq!(stat.grid[7], 10);
        assert_eq!(stat.grid[8], 10);
    }

    /// 差分的两格：自己对自己是 0；一格之差要**按绝对值**加进总数。
    #[test]
    fn the_grid_diff_is_absolute_and_zero_against_itself() {
        let mut one = vec![0_u64; 4];
        let mut two = vec![0_u64; 4];
        one[1] = 100;
        two[1] = 30;
        two[3] = 7;
        assert_eq!(grid_diff(&one, &one), 0);
        assert_eq!(grid_diff(&one, &two), 70 + 7);
        assert_eq!(grid_diff(&two, &one), 70 + 7, "差分是对称的");
    }

    /// 阈值两格：比例那一档（1% 的光通量）与**下界 1**（全黑参考图）。
    #[test]
    fn the_threshold_has_a_floor_of_one() {
        assert_eq!(cloud_threshold(0), 1, "全黑参考图不许把阈值降成 0");
        assert_eq!(cloud_threshold(1), 1);
        assert_eq!(cloud_threshold(100), 1);
        assert_eq!(cloud_threshold(1000), 10);
        assert_eq!(cloud_threshold(99), 1, "0.99 截断成 0 ⇒ 下界兜住");
    }

    /// 五条分支**各钉一次**（次序就是语义）。尤其是第一条：占位洋红的结论必须是
    /// "这一张不算数"，而不是"该有云却没画出来"。
    #[test]
    fn every_verdict_branch_says_its_own_reason() {
        let base = Verdict {
            declared: true,
            placeholder_px: 0,
            bright_px: 100,
            diff: 500,
            threshold: 100,
            is_reference: false,
        };
        let (has_cloud, text) = cloud_verdict(&base);
        assert!(has_cloud && text.starts_with("有云 ✓"), "{text}");

        let placeholder = Verdict {
            placeholder_px: 7,
            ..base
        };
        let (has_cloud, text) = cloud_verdict(&placeholder);
        assert!(!has_cloud && text.contains("占位 shader"), "{text}");

        let bare = Verdict {
            declared: false,
            ..base
        };
        let (has_cloud, text) = cloud_verdict(&bare);
        assert!(!has_cloud && text.contains("无云档"), "{text}");

        let reference = Verdict {
            is_reference: true,
            ..base
        };
        let (has_cloud, text) = cloud_verdict(&reference);
        assert!(has_cloud && text.contains("参考图"), "{text}");

        let thin = Verdict { diff: 99, ..base };
        let (has_cloud, text) = cloud_verdict(&thin);
        assert!(!has_cloud && text.contains("该有云却没画出来"), "{text}");
    }

    /// 参考图那一档的判据是"亮像素 > 0"：全黑（画了个寂寞）必须**不算**有云。
    #[test]
    fn a_reference_that_drew_nothing_is_not_a_cloud() {
        let dark = Verdict {
            declared: true,
            placeholder_px: 0,
            bright_px: 0,
            diff: 0,
            threshold: 1,
            is_reference: true,
        };
        let (has_cloud, text) = cloud_verdict(&dark);
        assert!(!has_cloud, "{text}");
    }
}
