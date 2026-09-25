pub const GRID_COLS: u32 = 16;
pub const GRID_ROWS: u32 = 10;
pub const BRIGHT_SUM: u32 = 72;
pub const CLOUD_DIFF_RATIO: f64 = 0.01;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotStat {
    pub placeholder_px: u64,
    pub bright_px: u64,
    pub grid: Vec<u64>,
}

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

pub fn grid_diff(one: &[u64], two: &[u64]) -> u64 {
    one.iter()
        .zip(two.iter())
        .map(|(left, right)| left.abs_diff(*right))
        .sum()
}

pub fn cloud_threshold(reference_luma: u64) -> u64 {
    ((CLOUD_DIFF_RATIO * reference_luma as f64) as u64).max(1)
}

pub struct Verdict {
    pub declared: bool,
    pub placeholder_px: u64,
    pub bright_px: u64,
    pub diff: u64,
    pub threshold: u64,
    pub is_reference: bool,
}

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

    #[test]
    fn the_three_readings_have_their_edges_pinned() {
        let (width, height) = (16_u32, 10_u32);
        let mut pixels = canvas(width, height);
        put(&mut pixels, width, 0, 0, [255, 3, 255]);
        put(&mut pixels, width, 1, 0, [255, 8, 255]);
        put(&mut pixels, width, 2, 0, [24, 24, 24]);
        put(&mut pixels, width, 3, 0, [24, 24, 25]);
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

    #[test]
    fn the_grid_uses_height_for_rows_and_width_for_columns() {
        let (width, height) = (960_u32, 640_u32);
        let mut pixels = canvas(width, height);
        put(&mut pixels, width, 0, 319, [10, 0, 0]);
        put(&mut pixels, width, 1, 319, [5, 0, 0]);
        put(&mut pixels, width, 0, 320, [10, 0, 0]);
        put(&mut pixels, width, 479, 0, [10, 0, 0]);
        put(&mut pixels, width, 480, 0, [10, 0, 0]);
        let stat = measure(&pixels, width, height);
        assert_eq!(stat.grid[4 * 16], 15, "同一格里的两个像素要累加");
        assert_eq!(stat.grid[5 * 16], 10);
        assert_eq!(stat.grid[7], 10);
        assert_eq!(stat.grid[8], 10);
    }

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

    #[test]
    fn the_threshold_has_a_floor_of_one() {
        assert_eq!(cloud_threshold(0), 1, "全黑参考图不许把阈值降成 0");
        assert_eq!(cloud_threshold(1), 1);
        assert_eq!(cloud_threshold(100), 1);
        assert_eq!(cloud_threshold(1000), 10);
        assert_eq!(cloud_threshold(99), 1, "0.99 截断成 0 ⇒ 下界兜住");
    }

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
