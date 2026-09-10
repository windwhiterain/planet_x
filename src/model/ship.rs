use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, CityId, FactionId, ShipId};

/// 一艘舰的「行为风格」——自动控制(`autocontrol`)读取它来决定怎么打。每条轴取 `[-1,1]`，
/// **0 = 基线**(与旧行为一致)。这是 **per-舰** 的配置,不是全局值:舰出厂时继承所属舰级的
/// [`ShipSpec::default_doctrine`],也可由 `--apply` 按舰覆写。引擎本身不读它——它只影响
/// 「AI 想怎么打」的决策,是自动控制模块的输入。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, JsonSchema)]
pub struct ShipDoctrine {
    /// 理智<->热血 (欺软怕硬<->飞蛾扑火): `<0` 倾向攻击威慑**低于**自己的目标;`>0` 倾向
    /// 攻击威慑**高于**自己的目标。0 = 基线(无视威慑,按基本权重选目标)。
    #[serde(default)]
    pub temper: f64,
    /// 护航<->独狼: `<0` 空闲舰贴旗舰护航(结伴);`>0` 空闲舰独自就近接战(独狼)。
    /// 0 = 基线(按配置的护航半径)。〈风筝<->贴脸〉已从行为风格降级为普通舰船控制属性
    /// [`Ship::kiting`]。
    #[serde(default)]
    pub lone_wolf: f64,
}
/// A ship's controllable behavior — the instruction a faction issues to one
/// of its ships. This is command-controlled state (see [`ControllableState`]),
/// not an event: the simulation merely reads this to decide where to move and
/// what to fire/bombard.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum ShipBehavior {
    /// 目标地点：移动到指定位置。
    Move { position: [f64; 2] },
    /// 跟随舰船：持续驶向目标舰的当前位置。所随的舰**可以是友方**（护航/护卫）**也可以是
    /// 敌方**（追袭/接战）——跟随本身**不主动开火**；攻击/轰炸在射程内**自动**发生。任何
    /// 敌舰进入自身攻击半径都会自动开火，与行为无关。
    Follow { ship: ShipId },
    /// 停泊城市：驶向某城——若该城为敌对势力且进入围城射程则**自动轰炸**；否则仅停靠/巡航。
    DockCity { city: CityId },
    /// 停泊轨道：跟随某个天体——持续向该天体当前位置移动，随其轨道巡航/停靠。
    Dock { body: BodyId },
    /// 殖民：前往定居点天体并（再）建立一座城市。
    Colonize { body: BodyId },
    /// 待命（无指令，原地保持当前坐标——由 AI/玩家写入的默认值）。
    Idle,
}
/// A spaceship. Always owned by a faction.
///
/// `name` is the **unique identity** (the schema's authoritative key, per the
/// design philosophy: "name is the unique key"). There is deliberately no separate
/// numeric id — a single source of truth, no shadow structure.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Ship {
    pub name: String,
    pub class: String,
    pub faction_id: FactionId,
    /// Position in AU (same plane as the orbits).
    pub position: [f64; 2],
    /// Current hull (armor) — must never exceed [`Self::hull_max`].
    pub hull: f64,
    /// 本舰最大护甲（含组件加成）。`hull` 是当前值；再生/损毁以 `hull_max` 为上限。
    #[serde(default = "default_hull_max")]
    pub hull_max: f64,
    /// 当前能量护盾值（护盾优先吸收、每回合再生，见 `shield_regen`）。
    #[serde(default)]
    pub shield: f64,
    /// 最大能量护盾（护盾组件的 `shield` 加总；无护盾组件为 0）。
    #[serde(default)]
    pub shield_max: f64,
    /// 本舰装配的组件 id（舰船定制）。空 = 裸舰（仅按 class 基础面板）。
    /// 由模拟在造舰出厂时确定性挑选并扣成本；agent 可直读以了解舰队构成。
    #[serde(default)]
    pub components: Vec<String>,
    /// 每个组件的完整度（与 `components` 同下标）。战斗中被击中会「溢出」损坏组件——
    /// 完整度 ≤0 即该组件被击毁，不贡献面板/武器（渐进丧失战力，而非满血抗到壳破）。
    /// 空 = 旧数据/裸舰（视为全部完好）。
    #[serde(default)]
    pub component_hp: Vec<f64>,
    /// 当前速度（AU/月）：每回合按推进模块的 `accel` 提升、最多到巡航速度 `speed`。
    /// 体现「加速到巡航需要时间」——推进模块给的加速度决定多快抵达战术位置。
    #[serde(default)]
    pub velocity: f64,
    /// 本舰行为风格的**记录值**：出厂继承 `ShipSpec::default_doctrine`，之后是流水。
    ///
    /// ⚠ 它**不是**有效风格。风格是活层：`--apply` 写的是 [`ControllableState::ship_doctrine`]
    /// 的**叶片**，有效值走 `State::ship_doctrine`（叶 → 舰队默认 → **这个记录值**）。
    /// agent 视图（`--round`/`--traj`）与投影给的都是**有效值**；这里只在 checkpoint 与
    /// 内存状态里保留出厂快照。
    #[serde(default)]
    pub doctrine: ShipDoctrine,
    /// 本舰的「风筝<->贴脸」姿态的**记录值**（per-舰 普通控制属性，非行为风格）：`[-1,1]`，
    /// `<0` = 风筝（保持武器射程、敌近则拉开、更早撤），`>0` = 贴脸（贴近敌舰、打完再撤）。
    /// `0` = 基线。**软目标**：Move/Follow/Dock/Idle 都是软目标——附近有敌舰时此姿态会
    /// 自动调整本舰移动（对玩家与 AI 一视同仁），不构成硬命令。引擎结算不读它，仅自动控制读。
    ///
    /// ⚠ 与 [`Self::doctrine`] 一样，**它不是有效值**：有效姿态走 `State::ship_kiting`
    /// （叶 → 舰队默认 → 这个记录值）；agent 视图与投影给的都是有效值。
    #[serde(default)]
    pub kiting: f64,
    /// 本舰的攻击历史：目标舰名 -> 「最近被本舰攻击过」的新鲜度 (0..1)。每回合衰减；本舰
    /// 刚攻击某目标就把它的新鲜度刷新到 1。各武器的火力分配层据此**降低最近打过目标的
    /// 权重**（雨露均沾），聚焦武器则反向加权（死磕补刀）。空 = 无历史（基线）。
    #[serde(default)]
    pub attack_hist: BTreeMap<ShipId, f64>,
}

fn default_hull_max() -> f64 {
    0.0
}
/// A step coprime to `len`, used to scramble the round-robin walk so names read as
/// "random" while staying deterministic. Falls back to 1 (plain round-robin) if no
/// odd step below `len` is coprime to it.
fn coprime_step(len: usize) -> usize {
    if len <= 1 {
        return 1;
    }
    (3..len)
        .rev()
        .find(|&s| gcd(s, len) == 1)
        .unwrap_or(1)
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Deterministic, unique-per-faction ship name drawn from a faction's name pool.
///
/// `seq` is a per-faction monotonic counter (never reused, so a destroyed ship's
/// name is never handed to a replacement). The pool is walked in a *scrambled*
/// round-robin (coprime step) — so the first `pool.len()` ships use each word once in
/// a non-alphabetical order, and wrapping the pool appends a generation numeral to
/// keep the name unique. Pure function of `(pool, seq)`: no RNG, no shared state.
///
/// This is what makes the ship *name* a usable unique key, per the design philosophy.
pub fn ship_display_name(pool: &[String], seq: u64) -> String {
    if pool.is_empty() {
        return format!("舰-{}", seq + 1);
    }
    let len = pool.len();
    let step = coprime_step(len);
    let idx = ((seq as usize).wrapping_mul(step)) % len;
    let word = &pool[idx];
    let generation_num = seq / (len as u64);
    if generation_num == 0 {
        word.clone()
    } else {
        format!("{}{}", word, generation_num + 1)
    }
}
