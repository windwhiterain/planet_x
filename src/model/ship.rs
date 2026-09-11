use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, CityId, FactionId, ResourceMap, ShipId};

/// **一艘舰的角色**（第三条风格轴的**取值**）：自动控制按它决定「这艘舰本回合干哪种活」。
///
/// 用户裁决（`tech-system.md` §11 裁决 9b = (a)）：**角色轴加第三态**——原来是 `bool`
/// （运输/战斗），现在是三态：
///
/// | 值 | 自动控制派它干什么 | 为什么存在 |
/// |---|---|---|
/// | [`ShipRole::War`] | 找仗打：接战 / 轰炸 / 殖民 | 基线（旧 `false`） |
/// | [`ShipRole::Freight`] | 排集货路线（`autocontrol::freight`） | 旧 `true` |
/// | [`ShipRole::Observe`] | **去异常区蹲着**（`autocontrol::knowledge`）——它是 MOND 掌握度**唯一**的知识来源 | 裁决 9b：没有它，知识渠道空转（实测没人去拿） |
///
/// ⚠ 三态是**互斥**的（一条轴一个值），不是三条并行的 bool：一艘舰同一时刻只有一种活。
/// 优先级是**观测 > 运输 > 战斗**（`should_be_role` 的判据顺序）——观测排第一是因为
/// 「没有观测，渠道就空转」，而运输可以由别的舰补上。
///
/// ⚠ 它**不是有效值**：有效角色走 `State::ship_role`（叶 → 舰队默认 → **记录值**）。
/// 三态中的任何一个都**不解除武装**：观测舰、运输舰在射程内照样自动开火、照样按
/// `kiting` 姿态软移动（沿用「角色只管派哪种活」的既有裁决）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, JsonSchema)]
pub enum ShipRole {
    /// 战舰：找仗打（接战 / 轰炸 / 殖民）。
    #[default]
    War,
    /// 运输舰：跑集货路线（把产地货栈的货搬回首都）。
    Freight,
    /// **观测舰**：驻在引力异常区里，把「飞船在异常区」这条知识渠道喂给本势力。
    Observe,
}

impl ShipRole {
    /// 稳定短名（观察面/日志用；serde 的 JSON 形态与它一致）。
    pub fn name(self) -> &'static str {
        match self {
            Self::War => "War",
            Self::Freight => "Freight",
            Self::Observe => "Observe",
        }
    }
}

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
    /// **运输**：在 `from` 装货、运到 `to` 卸货，卸完**自动返回 `from` 再装**——它是一条
    /// **常驻路线**，不是一次性任务（带截止期的一次性运输留给将来的承包合同）。
    ///
    /// **当前跑在哪一腿不存状态，由货舱推出来**：舱里有货 ⇒ 驶向 `to`；空舱 ⇒ 驶向 `from`。
    /// 于是「去 `from` 装 → 去 `to` 卸 → 再回 `from`」是一段不需要任何额外字段的循环：
    /// 任何时刻的航向都只由 `(舰在哪儿, 舱里有货吗)` 决定。
    ///
    /// 装卸都要求舰**停在 `arrival_eps` 之内**（与殖民/停泊同一把尺子，且目标点同样会被
    /// MOND 偏移——所以深处取货是「多试几个回合」，不是「取不到」）。装的是**本势力在该
    /// 天体的产地货栈**（见 [`crate::model::State::depots`]）；卸货进 `to`——`to` 是本势力
    /// **首都**就直接进 [`crate::model::Faction::resources`]（公理：首都即集散地），
    /// 否则进该天体的货栈（中转）。
    Haul { from: BodyId, to: BodyId },
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
    #[serde(rename = "舰名")]
    pub name: String,
    #[serde(rename = "舰级")]
    pub class: String,
    #[serde(rename = "势力")]
    pub faction_id: FactionId,
    /// Current hull (armor) — must never exceed [`Self::hull_max`].
    #[serde(rename = "船体")]
    pub hull: f64,
    /// 本舰最大护甲（含组件加成）。`hull` 是当前值；再生/损毁以 `hull_max` 为上限。
    #[serde(default = "default_hull_max")]
    #[serde(rename = "船体上限")]
    pub hull_max: f64,
    /// 当前能量护盾值（护盾优先吸收、每回合再生，见 `shield_regen`）。
    #[serde(default)]
    #[serde(rename = "护盾")]
    pub shield: f64,
    /// 最大能量护盾（护盾组件的 `shield` 加总；无护盾组件为 0）。
    #[serde(default)]
    #[serde(rename = "护盾上限")]
    pub shield_max: f64,
    /// 当前速度（AU/月）：每回合按推进模块的 `accel` 提升、最多到巡航速度 `speed`。
    /// 体现「加速到巡航需要时间」——推进模块给的加速度决定多快抵达战术位置。
    #[serde(default)]
    #[serde(rename = "速度")]
    pub velocity: f64,
    /// 本舰行为风格的**记录值**：出厂继承 `ShipSpec::default_doctrine`，之后是流水。
    ///
    /// ⚠ 它**不是**有效风格。风格是活层：`--apply` 写的是 [`ControllableState::ship_doctrine`]
    /// 的**叶片**，有效值走 `State::ship_doctrine`（叶 → 舰队默认 → **这个记录值**）。
    /// agent 视图（`--round`/`--traj`）与投影给的都是**有效值**；这里只在 checkpoint 与
    /// 内存状态里保留出厂快照。
    #[serde(default)]
    #[serde(rename = "风格")]
    pub doctrine: ShipDoctrine,
    /// 本舰的「风筝<->贴脸」姿态的**记录值**（per-舰 普通控制属性，非行为风格）：`[-1,1]`，
    /// `<0` = 风筝（保持武器射程、敌近则拉开、更早撤），`>0` = 贴脸（贴近敌舰、打完再撤）。
    /// `0` = 基线。**软目标**：Move/Follow/Dock/Idle 都是软目标——附近有敌舰时此姿态会
    /// 自动调整本舰移动（对玩家与 AI 一视同仁），不构成硬命令。引擎结算不读它，仅自动控制读。
    ///
    /// ⚠ 与 [`Self::doctrine`] 一样，**它不是有效值**：有效姿态走 `State::ship_kiting`
    /// （叶 → 舰队默认 → 这个记录值）；agent 视图与投影给的都是有效值。
    #[serde(default)]
    #[serde(rename = "姿态")]
    pub kiting: f64,
    /// 本舰的**角色**记录值（见 [`ShipRole`]）：自动控制派它干哪种活——打仗 / 跑运输 /
    /// 蹲异常区观测。
    ///
    /// ⚠ 与 [`Self::doctrine`]/[`Self::kiting`] 一样**它不是有效值**：有效角色走
    /// `State::ship_role`（叶 → 舰队默认 → **这个记录值**，出厂时继承
    /// [`crate::model::ShipSpec::default_role`]）。
    ///
    /// **角色只管一件事：自动控制把哪种活当成它的。**
    /// 它**不解除武装**——无论哪种角色的舰，在射程内照样自动开火、照样按 kiting 姿态软移动。
    /// 换句话说：它不是「军舰/民船」的军备差别，而是**同一个舰长的三种活**。
    #[serde(default)]
    #[serde(rename = "角色")]
    pub role: ShipRole,
    /// 本舰装配的组件 id（舰船定制）。空 = 裸舰（仅按 class 基础面板）。
    /// 由模拟在造舰出厂时确定性挑选并扣成本；agent 可直读以了解舰队构成。
    #[serde(default)]
    #[serde(rename = "组件")]
    pub components: Vec<String>,
    /// 每个组件的完整度（与 `components` 同下标）。战斗中被击中会「溢出」损坏组件——
    /// 完整度 ≤0 即该组件被击毁，不贡献面板/武器（渐进丧失战力，而非满血抗到壳破）。
    /// 空 = 旧数据/裸舰（视为全部完好）。
    #[serde(default)]
    #[serde(rename = "组件耐久")]
    pub component_hp: Vec<f64>,
    /// **在舱货物**：这艘舰此刻实际装着什么、各多少（`资源 → 数量`）。这是**真实物理量**，
    /// 不是账面数字——它只能由装卸两个动作改变：
    ///
    /// * **装货**：从某势力在某天体的**产地货栈**（[`crate::model::State::depots`]）里扣，
    ///   总量不得超过有效舱容（见下）。
    /// * **卸货**：进目标天体——若那是卸货势力的**首都**，就直接进 [`crate::model::Faction::resources`]
    ///   （首都即集散地，见 `.agents/notes/freight-collection.md`）；否则进该天体的货栈。
    ///
    /// **有效舱容**不是常数：`舰级舱容 × hull / hull_max`——装甲被打掉的运输舰装得少
    /// （受伤的船不敢满载）。舰级舱容见 [`crate::model::ShipSpec::cargo`]，
    /// 折算见 [`crate::model::cargo_capacity`]。空 = 空舱（出厂/旧档）。
    #[serde(default)]
    #[serde(rename = "载货")]
    pub cargo: ResourceMap,
    /// Position in AU (same plane as the orbits).
    #[serde(rename = "坐标")]
    pub position: [f64; 2],
    /// 本舰**出厂所用**的设计图名（快照的溯源，也是「按舰级默认意图」那一层的查表键）。
    ///
    /// `None` = 无图（旧档 / 开局预置舰队 / 剧情赠舰）。
    ///
    /// ⚠ 它**不**表示「本舰的选装可以随图变化」——`components` 是**快照**
    /// （用户裁决 §3.1）。它的两个用途：
    /// * **归因**：这艘舰是哪张图造出来的（投影 `ships.blueprint` 列、读面 join 蓝图表）；
    /// * **意图是活层**（Q2=(b)）：舰上只记图名，取值时现查图——改图的 `order` 会立刻对
    ///   这张图的所有舰（指令叶沉默者）生效，而面板/组件仍是快照。
    #[serde(default)]
    #[serde(rename = "出厂图")]
    pub blueprint: Option<crate::model::BlueprintId>,
    /// 本舰的攻击历史：目标舰名 -> 「最近被本舰攻击过」的新鲜度 (0..1)。每回合衰减；本舰
    /// 刚攻击某目标就把它的新鲜度刷新到 1。各武器的火力分配层据此**降低最近打过目标的
    /// 权重**（雨露均沾），聚焦武器则反向加权（死磕补刀）。空 = 无历史（基线）。
    #[serde(default)]
    #[serde(rename = "攻击历史")]
    pub attack_hist: BTreeMap<ShipId, f64>,
    /// 本舰**下水所在回合**（建造漏斗 `spawn_ship` 写入；`None` = 旧档缺字段 ⇒ **未知**）。
    ///
    /// 用途：编制表/花名册的**确定性 tie-break**（「同分取最老的」——在此之前只能用名字序
    /// 当代理，而名字序与年龄无关）。旧档的舰一律是 `None`，读者要**回落名字序**，不能把
    /// 「未知」当成第 0 回合下水（那会让旧档里所有舰并列最老）。
    #[serde(default)]
    #[serde(rename = "下水回合")]
    pub spawned_round: Option<u32>,
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
    (3..len).rev().find(|&s| gcd(s, len) == 1).unwrap_or(1)
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
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
