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
pub fn move_toward(state: &mut State, config: &GameConfig, ship_id: &str, _class: &str, dest: [f64; 2]) {
    let Some(ship) = state.ship(ship_id).cloned() else { return };
    let pos = ship.position;
    let fid = ship.faction_id.clone();
    // MOND 异常区：没有掌握修正引力的势力把指令坐标「算错」，实际航向产生偏移。
    // 偏移幅度是**伪随机**的（`nav_roll` 按 势力×舰名×回合 派生）：这一回合偏多少是确定的，
    // 但**下回合是全新的一次尝试**——所以深处目标不是「进不去」，而是「要多试几个回合」。
    let dest = mond_drift(config, &fid, dest, nav_roll(&fid, &ship.name, state.round));
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
        let next = if accel > 1e-9 { ship.velocity + accel } else { cruise };
        next.min(cruise)
    } else {
        0.0
    };
    let step = if distance <= config.combat.arrival_eps { 0.0 } else { vel.min(distance) };
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
    if let Some(s) = state.ships.iter().find(|s| s.faction_id == fid && s.hull > 0.0) {
        return s.position;
    }
    [0.0, 0.0]
}
