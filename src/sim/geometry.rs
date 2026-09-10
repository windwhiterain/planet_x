//! 位置/距离/趋近等纯几何工具（不含导航掷骰，见 `mond`）。

use super::*;

pub fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

pub fn city_position(state: &State, cid: &str) -> [f64; 2] {
    state
        .city(cid)
        .map(|c| state.body_position(&c.body_id))
        .unwrap_or([0.0, 0.0])
}

/// C¹ 连续斜坡：`t = clamp01((x−a)/(b−a))`，`t²(3−2t)`。输出 0..1，端点无跳变。
pub fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Move a ship one round's step toward `dest`, capped by its class speed.
///
/// **B5**：`inputs` 收「这一回合这艘舰**偏航了多少**」（MOND 的 `nav_roll`，空盐那一档）——
/// 它是**主 `Prng` 之外**的派生骰子（`(势力, 舰名, 回合)` 的哈希），回答「为什么它没到指令
/// 坐标」。`None` = 这一趟不记账（只读/复算的调用方，例如测试与读面）。
pub fn move_toward(
    state: &mut State,
    config: &GameConfig,
    ship_id: &str,
    _class: &str,
    dest: [f64; 2],
    inputs: Option<&mut RoundInputs>,
) {
    let Some(ship) = state.ship(ship_id).cloned() else {
        return;
    };
    let pos = ship.position;
    let fid = ship.faction_id.clone();
    // MOND 异常区：势力把指令坐标「算错」多少，由它的**掌握度**连续决定（`control = 1`
    // 就是今天的崇拜教，指哪打哪）；偏移幅度还是**伪随机**的（`nav_roll` 按 势力×舰名×回合
    // 派生）：这一回合偏多少是确定的，但**下回合是全新的一次尝试**——所以深处目标不是
    // 「进不去」，而是「要多试几个回合」。
    let control = mond_control(state, &fid);
    let roll = nav_roll(&fid, &ship.name, state.round);
    let dest = mond_drift(config, control, dest, roll);
    // **输入面（B5）**：登记这次偏航——「指令坐标 vs 实际坐标」的差就是它造成的。
    if let Some(rec) = inputs {
        rec.push_roll(crate::model::Roll {
            purpose: "nav".to_string(),
            faction: fid.clone(),
            subject: ship.name.clone(),
            value: roll,
            // 导航没有「闸门」：它是一枚**幅度骰**（偏移 = roll 派生的量）。所以两个判据字段
            // 都留空，`picked` 记下**实际算出来的落点**——那才是「偏到哪儿去了」。
            threshold: None,
            pool_total: None,
            picked: Some(format!("{:.3},{:.3}", dest[0], dest[1])),
        });
    }
    let distance = dist(pos, dest);
    if distance <= 1e-9 {
        return;
    }
    // 有效速度 = 推进模块给的**巡航速度**（被舰级 speed_mult 缩放），但当前速度
    // `velocity` 每回合按推进模块的**加速度** `accel` 提升、最多到巡航——舰船不能
    // 瞬间加速，而是逐步逼近巡航速度（加速度=战位调整的快慢）。
    let panel = ship_panel(config, &ship);
    let cruise = panel.speed;
    // 当前速度向巡航逼近（accel 若为 0 则直接到巡航，避免推进全被打伤时卡死）。
    let vel = if cruise > 0.0 {
        let accel = panel.accel;
        let next = if accel > 1e-9 {
            ship.velocity + accel
        } else {
            cruise
        };
        next.min(cruise)
    } else {
        0.0
    };
    let step = if distance <= config.combat.arrival_eps {
        0.0
    } else {
        vel.min(distance)
    };
    if let Some(s) = state.ship_mut(ship_id) {
        s.velocity = vel;
        if step > 0.0 {
            let nx = (dest[0] - pos[0]) / distance;
            let ny = (dest[1] - pos[1]) / distance;
            s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
        }
    }
}

/// 某势力的**交割锚点**（货物从哪儿发出 / 运到哪儿）：优先其首都天体，其次（无活城的
/// 流亡势力）其第一艘活舰的位置，都没有则原点。用于估算贸易路线的距离与是否穿越异常带。
pub fn trade_anchor(state: &State, fid: &str) -> [f64; 2] {
    let cap = state.capital_body(fid);
    if !cap.is_empty() && state.body(&cap).is_some() {
        return state.body_position(&cap);
    }
    if let Some(s) = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid && s.hull > 0.0)
    {
        return s.position;
    }
    [0.0, 0.0]
}
