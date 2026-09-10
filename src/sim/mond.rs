//! MOND 异常导航：深处的偏移是**伪随机范围**，掷骰走 `(势力, 舰名, 回合, 用途)` 派生哈希。

use super::*;

/// 一条贸易路线**浸入引力异常带的最大深度**（AU，0 = 完全没进异常带）。
///
/// * 两端都在异常带外 → 0（普通航线，无 MOND 代价）。
/// * 一端在内一端在外 → `远端半径 − radius`（越深，穿越成本越高）。
/// * 两端都在带内 → `较浅那端半径 − radius`（整条线都在异常带里，走浅处算）。
///
/// 就是 `decider_radius - mond.radius` 的闭式表达，确定性、无 RNG。
pub fn route_depth(config: &GameConfig, a: [f64; 2], b: [f64; 2]) -> f64 {
    let r = |p: [f64; 2]| (p[0] * p[0] + p[1] * p[1]).sqrt();
    let (ra, rb) = (r(a), r(b));
    let (inner, outer) = if ra <= rb { (ra, rb) } else { (rb, ra) };
    let deep_ref = if inner > config.mond.radius {
        inner
    } else {
        outer
    };
    (deep_ref - config.mond.radius).max(0.0)
}

/// 势力当前的 **MOND 掌握度**（0..1，缺省 0 = 凡人）。这是科技体系的干线：
/// 它连续地决定异常区里「一次导航尝试的胜算」，而不是某个档位上的开关。
pub fn mond_control(state: &State, fid: &str) -> f64 {
    state
        .faction(fid)
        .map(|f| f.mond_control)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

/// **前沿海拔**：掌握度 `control` 的势力在异常区内能**一次到位**（`p = 1`）的最远日心距。
///
/// `r* = radius + arrival_eps / (drift_per_au × (1 − control))`，`control = 1` ⇒ 无穷。
/// 它是干线上最直观的读数（`0 → 30.0`、`0.35 → 31.1`、`0.70 → 34.7`、`0.90 → 48.0` AU）：
/// 前沿之外不是「进不去」，而是「期望要试 `1/p` 次」。
pub fn mond_frontier(config: &GameConfig, control: f64) -> f64 {
    let m = &config.mond;
    let mastery = 1.0 - control.clamp(0.0, 1.0);
    if m.drift_per_au <= 0.0 || mastery <= 0.0 {
        return f64::INFINITY;
    }
    m.radius + config.combat.arrival_eps / (m.drift_per_au * mastery)
}

/// MOND 主力导航偏移：`control` 是舰船所在势力的掌握度（见 [`mond_control`]）；
/// 目标点进入异常区（距太阳超过 `mond.radius`）时，返回一个沿切向偏移的**伪目标**。
/// 掌握度越低越难精确机动到深处目标（难以轰炸/殖民/停靠、也运不出货），
/// 体现「指令坐标与实际坐标产生偏移」。
///
/// **偏移幅度是伪随机的、不是写死的**（用户裁决，见 `.agents/notes/freight-collection.md`）：
/// `roll ∈ [0,1)`（由 [`nav_roll`] 按「势力 × 舰名 × 回合」确定性派生）决定这一次尝试
/// 偏多少——`roll = 0` 就是**指哪打哪**。掌握度连续化之后（`tech-system.md` §2）
/// 上界再乘 `(1 − control)`，于是：
///
/// ```text
/// 一次尝试的成功率  p = P(偏移 ≤ arrival_eps)
///                     = min(1, arrival_eps / (depth × drift_per_au × (1 − control)))
/// ```
///
/// **任何深度、任何掌握度都 p > 0**（哪怕深度 100 AU、MOND 再强，也只是多试几个回合，
/// 而不是永远进不去），而 p 随深度单调递减、随掌握度单调递增：
/// `伊克西翁 30.17 AU`（depth 2.17）→ 凡人 p≈0.92；`创神星 38.16`（depth 10.16）→
/// 凡人 0.20、掌握 0.35 时 0.30、0.70 时 0.66。`control = 1` ⇒ 偏移恒 0（今天的 cult）。
///
/// 确定性：`roll` 由调用方给（纯函数），不消费主 `Prng` 流。
pub fn mond_drift(config: &GameConfig, control: f64, dest: [f64; 2], roll: f64) -> [f64; 2] {
    let m = &config.mond;
    let mastery = 1.0 - control.clamp(0.0, 1.0);
    if m.drift_per_au <= 0.0 || mastery <= 0.0 {
        return dest;
    }
    let r = (dest[0] * dest[0] + dest[1] * dest[1]).sqrt();
    let depth = (r - m.radius).max(0.0);
    if depth <= 0.0 {
        return dest;
    }
    // 这一次尝试偏多少：幅度 = 上界 × (1 − 掌握度) × roll^shape（`roll = 0` ⇒ 精确命中）。
    // `drift_shape < 1` 让偏移偏向大值（更常迷航在远处），但**永远留着蒙对的可能**。
    let shape = if m.drift_shape > 0.0 {
        m.drift_shape
    } else {
        1.0
    };
    let drift = depth * m.drift_per_au * mastery * roll.clamp(0.0, 1.0).powf(shape);
    // 切向（垂直于径向），代表轨道力学计算错误。
    let inv = if r > 1e-9 { 1.0 / r } else { 0.0 };
    let tx = -dest[1] * inv;
    let ty = dest[0] * inv;
    [dest[0] + tx * drift, dest[1] + ty * drift]
}

/// 一次**导航尝试**的确定性伪随机数 ∈ `[0,1)`——[`mond_drift`] 的偏移幅度取自它。
///
/// 就是 [`derived_roll`] 取**空盐**的那一档（`(势力, 舰名, 回合)`）。空盐让字节序列与
/// `derived_roll` 出现之前**逐字相同**，所以已经实测过的 MOND 表（成功率 / 首次命中回合）
/// 不作废。为什么不用主 [`Prng`] 流、为什么「下一回合是一次全新尝试」，见 [`derived_roll`]。
pub fn nav_roll(fid: &str, ship: &str, round: u32) -> f64 {
    derived_roll(fid, ship, round, "")
}

/// FNV-1a 64：只为把几个字段混成一个种子，不需要密码学强度。
pub fn fnv1a(bytes: impl Iterator<Item = u8>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 派生一枚**确定性伪随机数** ∈ `[0,1)`：把 `(势力, 舰名, 回合, 用途)` 混成一个 64 位种子
/// （FNV-1a）后再取 [`Prng::unit`]。
///
/// # 为什么不用主 [`Prng`] 流
///
/// 那会让「某艘舰多试了一次」改变**整个世界后续的掷骰**：同种子可复现就退化成
/// 「只要舰队数量一变，后面全变」，存档续玩的随机位置也解释不通。派生骰子是**独立**的：
/// 同一回合、同一艘舰、同一用途的结果恒定，下一回合才是一枚新骰子。
///
/// # `salt` = 用途
///
/// 同一艘舰在同一回合需要**多枚互不相关的骰子**（导航偏移、选线抽签……）。不区分用途，
/// 「偏得远的舰」和「被派去远货栈的舰」就会被同一枚骰子绑在一起（可观测的相关性）。
/// **空盐 = 导航那一档**（[`nav_roll`]），字节序列保持原样。
///
/// 这是本仓库的默认做法（见 `AGENTS.md`「默认用概率分布」）：**概率 ≠ 不可复现**。
pub fn derived_roll(fid: &str, ship: &str, round: u32, salt: &str) -> f64 {
    let mut bytes: Vec<u8> = Vec::with_capacity(fid.len() + ship.len() + salt.len() + 10);
    bytes.extend_from_slice(fid.as_bytes());
    bytes.push(b'|');
    bytes.extend_from_slice(ship.as_bytes());
    bytes.push(b'@');
    bytes.extend_from_slice(&round.to_le_bytes());
    if !salt.is_empty() {
        bytes.push(b'#');
        bytes.extend_from_slice(salt.as_bytes());
    }
    Prng::from_state(fnv1a(bytes.into_iter())).unit()
}

/// [`mond_drift`] 一次尝试的**成功率**：`偏移 = 上界 × (1 − 掌握度) × roll^shape`，
/// `roll` 均匀 ∈ `[0,1)`，而「到达」要求偏移 ≤ `arrival_eps`，于是
///
/// ```text
/// p = min(1, (arrival_eps / (depth × drift_per_au × (1 − control)))^(1/shape))
/// ```
///
/// `shape = 1`（默认）时就是 `eps/(depth×drift×(1−control))`。只服务观察与守卫
/// （结算读 [`mond_drift`]）：**p 恒 > 0**，深度越大只会越难、需要越多次尝试，
/// 永远没有「进不去」；掌握度越高 p 越大，`control = 1` ⇒ 恒 1（指哪打哪）。
pub fn mond_arrival_chance(config: &GameConfig, depth: f64, control: f64) -> f64 {
    let m = &config.mond;
    let mastery = 1.0 - control.clamp(0.0, 1.0);
    if m.drift_per_au <= 0.0 || depth <= 0.0 || mastery <= 0.0 {
        return 1.0;
    }
    let ratio = config.combat.arrival_eps / (depth * m.drift_per_au * mastery);
    if ratio >= 1.0 {
        return 1.0;
    }
    let shape = if m.drift_shape > 0.0 {
        m.drift_shape
    } else {
        1.0
    };
    ratio.powf(1.0 / shape)
}

// --- 集货腿（M2b）：装卸货 + 运输路线的执行 -----------------------------------
