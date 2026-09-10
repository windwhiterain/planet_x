use serde::{Deserialize, Serialize};

use crate::model::{GameConfig, ResourceMap, Ship, ShipDoctrine, ShipRole};

/// A ship class keyed by name in the config. The mechanics reference this key
/// only through the config tables.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShipSpec {
    pub label: String,
    /// 船体 (hull points) —— **舰级直接属性**。模块**不能**改变它：护甲只是让船体变硬
    /// （减伤）、护盾提供吸收池，二者都不改这条基准。这是「船体 = 平台本体」。
    pub hull: f64,
    /// 船体再生 (%/时间): fraction of the max hull restored per round —— 同样直接、不被
    /// 模块改变（模块的再生走护盾再生 `shield_regen`）。
    pub hull_regen: f64,
    /// Build points required to finish this class at a shipyard.
    pub build_points: f64,
    /// Resource cost to fully build a ship of this class.
    pub build_cost: ResourceMap,
    /// Per-round maintenance (in market value / credits) per ship — a continuous
    /// sink that caps fleet growth and makes big fleets expensive to sustain.
    pub upkeep: f64,
    /// 组件槽位数：本舰级可同时装配多少个「定制组件」（武器/护盾/护甲/推进/辅助）。
    /// 这是「飞船定制化」的能力上限——更大的舰级能承载更多组件，把资源优势转化为战斗力。
    #[serde(default)]
    pub slots: u32,
    // --- 舰级 = 平台修正器：这些系数**缩放它所搭载的模块**，而不是给舰叠加独立的
    // 面板数字。舰级身份（护卫=快、战列=重火力、航母=超远程）就体现在系数分布上。
    // 速度/加速度/攻击距离没有舰级基础值——它们完全来自推进/武器模块，故「推进」和
    // `武器`一样是必须的（没有推进模块就没有速度）。 ---
    /// 护甲硬度修正：作用于护甲模块的 hardness（反比例减伤）。
    #[serde(default = "default_mult")]
    pub armor_mult: f64,
    /// 护盾池修正：作用于护盾模块的 shield（吸收池）。
    #[serde(default = "default_mult")]
    pub shield_mult: f64,
    /// 护盾再生修正：作用于护盾/辅助模块的 shield_regen。
    #[serde(default = "default_mult")]
    pub shield_regen_mult: f64,
    /// 速度修正：作用于推进模块的巡航速度 `speed`。
    #[serde(default = "default_mult")]
    pub speed_mult: f64,
    /// 加速度修正：作用于推进模块的加速度 `accel`（每回合提升当前速度，最多到巡航）。
    #[serde(default = "default_mult")]
    pub accel_mult: f64,
    /// 伤害修正：作用于武器模块的 damage。
    #[serde(default = "default_mult")]
    pub attack_mult: f64,
    /// 攻击距离修正：作用于武器模块的 range。
    #[serde(default = "default_mult")]
    pub range_mult: f64,
    /// 点防御修正：作用于点防御模块（近防炮阵」)的 intercept（拦截量）。
    /// 哨戒/玻璃大炮因需要成群导弹来袭时各自扛一点，点防修正可高于 1.0；它是「舰级 =
    /// 平台修正器」的一环——把所搭载的点防模块输出按舰型缩放，而非给舰叠加独立面板。
    #[serde(default = "default_mult")]
    pub pd_mult: f64,
    /// **舱容**：本舰级一次能装多少单位货（运输装货的上限，见 [`Ship::cargo`]）。
    ///
    /// 它是与「武器/护甲」**并列的第三类能力**，所以**不从 hull 推**：船体大 ≠ 能装
    /// （战列舰 30 船体只装 6，航母 20 船体装 20——航母的机库/货舱本来就是它的招牌）。
    /// 有效舱容还要乘战损折算 `hull / hull_max`，见 [`cargo_capacity`]。
    /// 0 = 不承运（可以装 0，即永远装不满；不是错误状态）。
    pub cargo: f64,
    /// 本舰级出厂时的**默认行为风格**（per-舰 配置的类默认）：舰出厂时继承这一份
    /// `ShipDoctrine`。全 0 = 基线（与旧行为一致）。`--apply` 可再按单舰覆写。
    #[serde(default)]
    pub default_doctrine: ShipDoctrine,
    /// 本舰级出厂时的**默认风筝<->贴脸姿态**（普通舰船控制属性，非行为风格）：舰出厂时
    /// 继承这一份 `kiting`。全 0 = 基线。`--apply` 可再按单舰覆写。
    #[serde(default)]
    pub default_kiting: f64,
    /// 本舰级出厂时的**默认角色**（[`ShipRole`]）。舰出厂时继承它，之后落在
    /// [`Ship::role`] 那个记录值上；有效角色走 [`crate::model::State::ship_role`]
    /// （叶 → 舰队默认 → 记录值），自动控制与玩家都可以改。
    ///
    /// 只有**航母**出厂就是运输舰（[`ShipRole::Freight`]）：它是唯一的散货船（舱容 20 =
    /// 5 艘驱逐），让「舰级身份」和「它天生该干的活」对上。其余舰级出厂是
    /// [`ShipRole::War`]（= 旧行为不变；旧档的 serde 缺省也是它）。
    #[serde(default)]
    pub default_role: ShipRole,
}

fn default_mult() -> f64 {
    1.0
}

fn default_fire_rate() -> f64 {
    1.0
}
/// A ship component (舰船定制组件) keyed by id in the config. Fitting a component
/// onto a ship (during production) **changes what the ship can actually do in
/// combat**, not just a stat delta:
///
/// * weapon systems (`category="weapon"`) introduce a weapon with its own damage
///   **type** (kinetic/plasma/missile), engagement `range`, `tracking` (how well it
///   leads a fast target), and shield/hull multipliers. Choosing *which* weapons
///   you fit decides how you fight — so a 铀-rich faction fields anti-armor railguns,
///   a 金/铂-rich one fields shields + plasma, a 氦-3/氢-rich one fields fast
///   missile skirmishers. This is the resource→military link, and it is qualitative.
/// * defense (`category="defense"`) adds an energy **shield pool** (soaks damage
///   preferentially, regenerates), extra **hull** armor, or **point-defense**
///   interceptors that shoot down incoming missiles.
/// * thrust / utility (`category="thrust"|"utility"`) add speed, regen, etc.
///
/// The offensive baseline (a ship's class gun battery) is always present; the
/// fitted components are layered on top. All values are deterministic (no RNG in
/// combat; evasion is a deterministic function of target speed vs weapon tracking).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ComponentSpec {
    pub label: String,
    /// 组件类别：`"weapon"` / `"defense"` / `"thrust"` / `"utility"`——决定它在
    /// 战斗里扮演的角色（类别不直接进数值，数值由下列字段决定）。
    pub category: String,
    /// 武器伤害（非武器为 0）。对舰/对城都按「伤害类型 × 防御」结算。
    pub damage: f64,
    /// 武器伤害类型：`"kinetic"` / `"plasma"` / `"missile"`；非武器为 `""`。
    pub damage_type: String,
    /// 武器交战距离（AU）：只有目标进入此距离该武器才参与齐射——所以「射程」是
    /// 真实的战位选择，而非一个加分项。
    pub range: f64,
    /// 武器追踪能力（AU/月）：越高的武器越能咬住高速目标（跑得快的船对低追踪
    /// 武器规避强）。deterministic 命中 = 1 - evasion(target_speed, tracking)。
    pub tracking: f64,
    /// 武器对能量护盾的伤害倍率（护盾先吸收；kinetic×低、plasma×高）。
    pub shield_mult: f64,
    /// 武器对船体（装甲）的伤害倍率。
    pub hull_mult: f64,
    /// 武器射速（发/时间）：每回合该武器**独立**发射这么多发。**每一发独立索敌**——
    /// 单回合内多发可以火力分配（打不同目标），是「武器 = 独立索敌单位」的机制载体。
    /// 非武器组件（防御/推进/辅助）此值无意义（默认 1，但它们不开火）。
    #[serde(default = "default_fire_rate")]
    pub fire_rate: f64,
    /// 武器火力分配 单一目标<->雨露均沾（**武器自身可控属性**，非舰船风格）：`<0` 每发
    /// 都死磕一个目标（越近打过的越加分，集中火力补刀）；`>0` 越近的攻击历史越**降低**
    /// 那目标权重、把多发摊给不同目标；`0` = 不偏置（攻击历史不改目标权重）。它工作在
    /// 目标选择**基本权重**（距离 + 克制 + per-武器噪声）之上。
    #[serde(default)]
    pub fire_spread: f64,
    /// 能量护盾池（防御组件加）：0 = 无护盾。护盾池每回合再生。
    pub shield: f64,
    /// 护盾再生（占 shield_max / 月）。
    pub shield_regen: f64,
    /// 船体硬度（数值）：按**反比例函数**削减打向船体的伤害——「护甲让船体变硬」，
    /// 是减伤系数，**不是加血**。舰级的 base hull 是直接属性，模块不能改它。
    #[serde(default)]
    pub hardness: f64,
    /// 点防御拦截强度（数值）：**线性**扣减来袭导弹的伤害（拦掉 `intercept` 点）。
    pub intercept: f64,
    /// 推进模块的**巡航速度**（AU/月）：由舰级 `speed_mult` 缩放。没有推进就没速度。
    pub speed: f64,
    /// 推进模块的**加速度**（AU/月²）：每回合把当前速度提升这么多，最多到巡航速度
    /// `speed`——由舰级 `accel_mult` 缩放。体现「加速到巡航需要时间」。
    #[serde(default)]
    pub accel: f64,
    /// 额外船体再生（占 hull_max / 月）。
    pub hull_regen: f64,
    /// 额外每回合维护费（市场价值/舰）。组件越强，舰队越难养——抑制无脑堆强组件。
    pub upkeep: f64,
    /// 一次性装配成本（在造舰出厂时从势力库存扣除）。绑定资源优势的落点。
    pub cost: ResourceMap,
}
/// A concrete weapon a ship fields (its class battery plus any fitted weapon
/// components). `Copy` so the hot combat path iterates it cheaply.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Weapon {
    /// 单发齐射伤害。
    pub damage: f64,
    /// 交战距离（AU）。
    pub range: f64,
    /// 追踪能力（AU/月）——越高越难被高速目标规避。
    pub tracking: f64,
    /// 对护盾伤害倍率。
    pub shield_mult: f64,
    /// 对船体伤害倍率。
    pub hull_mult: f64,
    /// 伤害类型（0=kinetic, 1=plasma, 2=missile）。
    pub kind: u8,
    /// 射速（发/时间），来自组件；每发独立索敌。
    pub fire_rate: f64,
    /// 火力分配 单一目标<->雨露均沾：来自组件；作用于本武器每发的目标选择（见
    /// [`ComponentSpec::fire_spread`]）。
    pub fire_spread: f64,
    /// 本武器实例的稳定身份（由舰名 + 组件 id + 组件序号哈希），用于 per-weapon
    /// 确定性噪声——每件武器各有一点点不同的目标偏好。
    pub seed: u64,
}
/// The effective combat panel of a ship after its fitted components are applied.
/// Aggregated so movement / upkeep / regen read one struct; per-weapon resolution
/// (which weapon fires, at what damage, vs shield vs hull) is handled separately by
/// [`ship_weapons`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ShipPanel {
    /// 全武器模块的总攻击（用于对城轰炸的简化结算与报告）。
    pub attack: f64,
    /// 有效攻击距离：`max(武器.range × range_mult)`（来自武器模块，无舰级基础值）。
    pub attack_range: f64,
    /// 巡航速度：`Σ(推进.speed × speed_mult)`（来自推进模块，无舰级基础值）。
    pub speed: f64,
    /// 加速度：`Σ(推进.accel × accel_mult)`（来自推进模块）。
    pub accel: f64,
    /// 舰级的 base hull（直接属性，模块不改）。
    pub hull_max: f64,
    /// 舰级的 base hull_regen（直接属性，模块不改）。
    pub hull_regen: f64,
    pub shield_max: f64,
    pub shield_regen: f64,
    /// 点防御拦截强度（线性，直接扣减来袭导弹伤害）。
    pub intercept: f64,
    /// 舰体硬度（来自护甲组件，反比例减伤）：削减打向船体的伤害。
    pub hardness: f64,
    pub upkeep: f64,
}
/// Damage type tags used in weapon resolution.
pub const WEAPON_KINETIC: u8 = 0;
pub const WEAPON_PLASMA: u8 = 1;
pub const WEAPON_MISSILE: u8 = 2;

fn weapon_kind(t: &str) -> u8 {
    match t {
        "plasma" => WEAPON_PLASMA,
        "missile" => WEAPON_MISSILE,
        _ => WEAPON_KINETIC,
    }
}

/// 弹种 id → 可读名（[`weapon_kind`] 的逆）。用于事件里记「被什么打沉的」。
pub fn weapon_kind_name(kind: u8) -> &'static str {
    match kind {
        WEAPON_PLASMA => "plasma",
        WEAPON_MISSILE => "missile",
        _ => "kinetic",
    }
}

/// Compute a ship's effective combat panel from its class spec plus its fitted
/// components. Pure & deterministic (no RNG); cheap enough for the hot loop.
/// A component's 完整度 (integrity) when freshly installed: weapons/shields/armor each
/// add to how long it survives battle damage, with a floor so trivial parts are not
/// one-shot.
pub fn component_integrity(config: &GameConfig, id: &str) -> f64 {
    let cs = config.component_spec(id);
    (cs.shield + cs.hardness * 40.0 + cs.damage * 0.4).max(18.0)
}

/// A ship's component at index `i`'s effectiveness (0..1): its current integrity
/// fraction. A damaged module contributes less (a shot-out weapon fires weaker) —
/// the mechanical basis of gradual combat degradation. A missing `component_hp`
/// (legacy / bare ships) is fully effective; an emptied one is dead (0). 
pub fn component_effectiveness(config: &GameConfig, ship: &Ship, i: usize) -> f64 {
    let Some(hp) = ship.component_hp.get(i) else {
        return 1.0; // 无完整度记录（裸舰/旧数据）：视为完好。
    };
    if ship.components.get(i).map(|c| component_integrity(config, c)).unwrap_or(1.0) <= 1e-9 {
        return 1.0;
    }
    (hp / component_integrity(config, &ship.components[i])).clamp(0.0, 1.0)
}

/// 某舰的**有效舱容**（一次能装多少单位货）：舰级舱容 × **战损折算** `hull / hull_max`。
///
/// * 舰级舱容来自 [`ShipSpec::cargo`]——船体大小不决定它能装多少（见那里的说明）。
/// * 战损折算：装甲被打掉的运输舰装得少（`hull/hull_max` 越低，货舱越「不敢装满」）。
///   这是**连续**的：不用「受伤就罢工」这种硬阈值，而是让它运得少。
/// * `hull_max ≤ 0`（旧档缺该字段）按**未受损**处理（满舱）——否则 `hull/0` 会把舱容
///   算成无穷，让旧档的船变成无底洞。
///
/// 纯函数、无 RNG、不读 `state`：给定 `(config, ship)` 恒定。
pub fn cargo_capacity(config: &GameConfig, ship: &Ship) -> f64 {
    let cap = config.ship_spec(&ship.class).cargo;
    if ship.hull_max <= 0.0 {
        return cap;
    }
    cap * (ship.hull / ship.hull_max).clamp(0.0, 1.0)
}

/// 在舱货物的**总件数**（各资源求和）——与 [`cargo_capacity`] 同一把尺子（都是「单位」，
/// 不折算价值）。装货判「还能装多少」用它；卸货不设上限（货栈/池子有多大收多大）。
pub fn cargo_used(cargo: &ResourceMap) -> f64 {
    cargo.values().sum()
}

pub fn ship_panel(config: &GameConfig, ship: &Ship) -> ShipPanel {
    let base = config.ship_spec(&ship.class);
    // 舰级 = 平台修正器 + 直接船体属性。船体值(hull/hull_regen)是**直接**的，模块不改它；
    // 其余战斗属性（攻击/射程/速度/护盾/硬度/点防）全部由模块贡献、被舰级修正系数缩放。
    // 所以「推进」和「武器」一样是必须的：没有推进模块就没有速度、没有武器就没有攻击力。
    let mut p = ShipPanel {
        attack: 0.0,
        attack_range: 0.0,
        speed: 0.0,
        accel: 0.0,
        hull_max: base.hull,
        hull_regen: base.hull_regen,
        shield_max: 0.0,
        shield_regen: 0.0,
        intercept: 0.0,
        hardness: 0.0,
        upkeep: base.upkeep,
    };
    for (i, c) in ship.components.iter().enumerate() {
        let eff = component_effectiveness(config, ship, i);
        if eff <= 1e-9 {
            continue; // 彻底被击毁的组件不再贡献面板。
        }
        if let Some(cs) = config.components.get(c) {
            match cs.category.as_str() {
                "weapon" => {
                    p.attack += cs.damage * base.attack_mult * eff;
                    let r = cs.range * base.range_mult;
                    if r > p.attack_range {
                        p.attack_range = r;
                    }
                }
                "defense" => {
                    // 护盾池：吸收伤害，被 shield_mult 缩放。
                    p.shield_max += cs.shield * base.shield_mult * eff;
                    p.shield_regen += cs.shield_regen * base.shield_regen_mult * eff;
                    // 护甲：让船体变硬（反比例减伤），被 armor_mult 缩放。
                    p.hardness += cs.hardness * base.armor_mult * eff;
                    // 点防御：线性拦截导弹，被舰级点防御修正 pd_mult 缩放。
                    p.intercept += cs.intercept * base.pd_mult * eff;
                    // 一些防御组件也带附加速度（被 speed_mult 缩放）。
                    p.speed += cs.speed * base.speed_mult * eff;
                }
                "thrust" => {
                    // 推进 = 速度/加速度的**直接来源**，被舰级 speed_mult/accel_mult 缩放。
                    // 没有推进模块就没有速度。
                    p.speed += cs.speed * base.speed_mult * eff;
                    p.accel += cs.accel * base.accel_mult * eff;
                }
                "utility" => {
                    p.shield_regen += cs.shield_regen * base.shield_regen_mult * eff;
                    p.speed += cs.speed * base.speed_mult * eff;
                    p.accel += cs.accel * base.accel_mult * eff;
                }
                _ => {}
            }
            p.upkeep += cs.upkeep * eff;
        }
    }
    p
}

/// Enumerate the weapons a ship fields: each fitted weapon component. `kind` marks
/// the damage type so the combat resolver can apply shield/hull multipliers and
/// missile interception. There is **no class gun battery** — a ship's firepower is
/// entirely the weapon modules it carries, scaled by its class `attack_mult`.
pub fn ship_weapons(config: &GameConfig, ship: &Ship) -> Vec<Weapon> {
    let base = config.ship_spec(&ship.class);
    let mut ws = Vec::new();
    for (i, c) in ship.components.iter().enumerate() {
        let eff = component_effectiveness(config, ship, i);
        if eff <= 1e-9 {
            continue; // 彻底被击毁的武器组件不再开火。
        }
        if let Some(cs) = config.components.get(c) {
            if cs.category == "weapon" && cs.damage > 0.0 {
                ws.push(Weapon {
                    damage: cs.damage * base.attack_mult * eff,
                    range: cs.range * base.range_mult,
                    tracking: cs.tracking,
                    shield_mult: cs.shield_mult,
                    hull_mult: cs.hull_mult,
                    kind: weapon_kind(&cs.damage_type),
                    fire_rate: cs.fire_rate,
                    fire_spread: cs.fire_spread,
                    seed: weapon_seed(&ship.name, c, i),
                });
            }
        }
    }
    ws
}

/// 本武器实例的稳定身份：由（舰名、组件 id、组件序号）哈希出一个 u64。舰名与组件表都
/// 在回合间不变，故每件武器各有一个**确定性 noise 种子**——不同武器对同一目标各带一点
/// 不同的偏好，舰队火力才不会整齐划一。
fn weapon_seed(ship: &str, component: &str, idx: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in ship.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    for b in component.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h = (h ^ (idx as u64)).wrapping_mul(0x100_0000_01b3);
    h
}
