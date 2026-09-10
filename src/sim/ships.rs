//! 舰船：下水与蓝图装载、行为合法性、行为目的地、单舰战力与威慑。

use super::*;

/// 造一艘舰的参数（[`spawn_ship`] 的输入）。
pub struct ShipSpawn<'a> {
    pub owner: FactionId,
    pub class: &'a str,
    pub position: [f64; 2],
    /// 出厂城（船坞建造时给出；剧情赠舰 / 重建种子舰在天体附近下水则为 `None`）。
    pub city: Option<CityId>,
    pub via: SpawnVia,
    /// 是否从库存支付装配组件成本（船坞出厂要付；剧情赠舰与重建种子舰不付）。
    pub pay_components: bool,
    /// 出厂所用的**设计图名**（船坞那条路传该建造区的图；剧情赠舰传 `None`）。
    ///
    /// 它同时决定两件事：**装什么**（图的归属解析为 `Player` 且 `components` 非空 ⇒ 就是它，
    /// 否则现算 `choose_loadout`）与**归因**（写进 `Ship.blueprint`，投影/读面据此 join）。
    pub blueprint: Option<&'a BlueprintId>,
}

/// 造一艘舰（漏斗）：装配组件 + 确定性取名 + 算 effective 面板 + 记一条
/// [`GameEvent::ShipSpawned`]。
///
/// 三条造舰路径（船坞出厂 / 剧情赠舰 / 反僵尸重建）统一走这里。此前 `step_resurgence`
/// 的种子舰**完全不发事件**——投影实测 63 次出生里 **46 次无解释**，直到把对账守卫从
/// 「城的归属」扩到「舰的出生」才被抓出来（见 `every_ship_state_change_is_explained_by_an_event`）。
///
/// 确定性：`choose_loadout` / `ship_display_name` 都无 RNG，`ship_name_seq` 单调递增，
/// 故本漏斗不改变模拟的随机数流。**设计图只改「装什么」**：无图（旧档、预置舰队、剧情赠舰）
/// 与 `Auto` 图都走**同一个** `choose_loadout`、**同一时点**（出厂那一刻按当时库存算）。
///
/// **「当时库存」= 出厂城所在天体的库存**（[`State::stock_at`]：首都 ⇒ 池子、其余 ⇒ 货栈）：
/// 选装与付款读的是**同一处**，所以「选了装不起的模块、`commit_spend` 钳零白送」这条
/// 凭空造物的路被堵死（没有城的天体级下水——剧情赠舰 / 重建种子舰——算在首都）。
pub fn spawn_ship(state: &mut State, config: &GameConfig, spec: ShipSpawn<'_>) -> ShipId {
    let site: BodyId = spec
        .city
        .as_ref()
        .and_then(|c| state.city(c).map(|c| c.body_id.clone()))
        .unwrap_or_else(|| state.capital_body(&spec.owner));
    let stock: ResourceMap = state.stock_at(&spec.owner, &site).cloned().unwrap_or_default();
    let components =
        autocontrol::resolve_loadout(state, config, spec.owner.clone(), spec.class, spec.blueprint, &stock);
    let class = spec.class.to_string();
    let cspec = config.ship_spec(&class);
    // 舰名 = 从本势力名字库确定性取的一个唯一名（名字即唯一 key，击毁后不复用）。
    let seq = *state.ship_name_seq.entry(spec.owner.clone()).or_insert(0);
    state.ship_name_seq.insert(spec.owner.clone(), seq + 1);
    let fname = state.faction(&spec.owner).map(|f| f.name.clone()).unwrap_or_default();
    let name = ship_display_name(config.ship_pool(&fname), seq);
    let mut ship = Ship {
        name: name.clone(),
        class: class.clone(),
        faction_id: spec.owner.clone(),
        position: spec.position,
        hull: 0.0,
        hull_max: 0.0,
        shield: 0.0,
        shield_max: 0.0,
        components,
        component_hp: Vec::new(),
        velocity: 0.0,
        doctrine: cspec.default_doctrine,
        kiting: cspec.default_kiting,
        freighter: cspec.default_freighter,
        attack_hist: BTreeMap::new(),
        cargo: BTreeMap::new(),
        // 出厂归因：这艘舰是哪张图印出来的（`None` = 无图）。快照的溯源，不是活层。
        blueprint: spec.blueprint.cloned(),
        // 下水回合（编制表的「同分取最老的」靠它；旧档缺字段 ⇒ `None` = 未知）。
        spawned_round: Some(state.round),
    };
    let panel = ship_panel(config, &ship);
    ship.hull = panel.hull_max;
    ship.hull_max = panel.hull_max;
    ship.shield = panel.shield_max;
    ship.shield_max = panel.shield_max;
    // 每件组件初始满完整度（模块毁损用）。
    ship.component_hp = ship.components.iter().map(|c| component_integrity(config, c)).collect();
    if spec.pay_components {
        let comp_cost: Vec<(String, f64)> = ship
            .components
            .iter()
            .flat_map(|c| config.component_spec(c).cost.clone())
            .collect();
        let mut spent: ResourceMap = ResourceMap::new();
        commit_spend(state, &spec.owner, &site, &mut spent, &comp_cost);
    }
    ev(state, GameEvent::ShipSpawned {
        ship: name.clone(),
        owner: spec.owner.clone(),
        class: class.clone(),
        city: spec.city,
        via: spec.via,
        blueprint: spec.blueprint.cloned(),
    });
    state
        .control
        .entry(spec.owner)
        .or_default()
        .ship_orders
        .insert(name.clone(), Control::inherit(ShipBehavior::Idle));
    state.ships.push(ship);
    name
}

/// 威慑（自动计算）：某舰的威慑 = 本舰**综合战力** + `deterrence_radius` 内同势力友舰
/// 战力之和——「威慑 = 综合战力 + 附近同势力战力互相叠加」。这是「理智<->热血」选目标的
/// 依据：欺软怕硬打威慑低于自己的、飞蛾扑火打威慑高于自己的。确定性（无 RNG）。
pub fn deterrence(state: &State, config: &GameConfig, ship_id: &str) -> f64 {
    let Some(me) = state.ship(ship_id) else { return 0.0 };
    let r = config.combat.deterrence_radius;
    let mut d = ship_power(config, me);
    if r > 0.0 {
        for s in &state.ships {
            if s.name != ship_id && s.faction_id == me.faction_id && s.hull > 0.0
                && dist(me.position, s.position) <= r {
                d += ship_power(config, s);
            }
        }
    }
    d
}

/// 综合战力：把一艘舰的当前有效面板折算成一个标量（威慑 / 目标价值用）。权重与选装评分
/// 一致（攻击最重、护甲/点防次之），让「威慑」是真实的火力估值而非拍脑袋。
pub fn ship_power(config: &GameConfig, ship: &Ship) -> f64 {
    let p = ship_panel(config, ship);
    p.attack * 4.0 + p.hull_max * 1.0 + p.shield_max * 0.8 + p.hardness * 3.0 + p.intercept * 2.0
}

pub fn behavior_is_valid(state: &State, _config: &GameConfig, behavior: ShipBehavior, owner: &str) -> bool {
    match behavior {
        ShipBehavior::Move { .. } | ShipBehavior::Idle => true,
        ShipBehavior::Dock { body } => state.body(&body).is_some(),
        // 跟随舰船：所随舰还活着即可（友方护航 / 敌方追袭皆可）。
        ShipBehavior::Follow { ship } => state.ship(&ship).map(|s| s.hull > 0.0).unwrap_or(false),
        // 停泊城市：城还活着即可停靠/包围；是敌城则会在围城射程内自动轰炸。
        ShipBehavior::DockCity { city } => state.city(&city).map(|c| !c.razed).unwrap_or(false),
        ShipBehavior::Colonize { body } => has_blank_site(state, &body),
        // 运输路线：两端天体都要在。**`from == to` 只在「卸进首都池」时合法**——
        // 「自取自卸」没有意义（那是陈旧指令），但「首都天体上压着一处旧中转货栈、
        // 把它扫进池子」是合法的路线（迁都把旧中转点留在了新首都，见 `autocontrol::freight`）。
        ShipBehavior::Haul { from, to } => {
            state.body(&from).is_some()
                && state.body(&to).is_some()
                && (from != to || state.capital_body(owner) == to)
        }
    }
}

/// 一次性指令（殖民）执行完之后的收尾：指令复位成 `Idle`，但**保留「由谁决定」**。
///
/// 为什么必须保留：殖民是「命令 → 执行 → 指令失效」的一次性动作，而这里以前无条件写
/// `Control::inherit(..)`，于是**玩家点名的殖民舰一旦建完城就被交还给系统**（AI 下一回合
/// 就把它征去别处）——玩家会看到自己刚下达的处置静默蒸发。换「值」不换「归属」是 sim 里
/// 的既有约定（见 `step_capital` 的迁都：保留原来的 mode 标记）。
pub fn reset_order_keep_mode(state: &mut State, fid: &FactionId, ship_id: &str) {
    if let Some(c) = state.control_mut(fid.clone()) {
        let mode = c.ship_orders.get(ship_id).map(|c| c.mode).unwrap_or_default();
        c.ship_orders.insert(ship_id.to_string(), Control { value: ShipBehavior::Idle, mode });
    }
}

pub fn behavior_dest(state: &State, behavior: &ShipBehavior) -> [f64; 2] {
    match behavior {
        ShipBehavior::Move { position } => *position,
        ShipBehavior::Follow { ship } => state.ship(ship).map(|s| s.position).unwrap_or([0.0, 0.0]),
        ShipBehavior::DockCity { city } => city_position(state, city),
        ShipBehavior::Dock { body } | ShipBehavior::Colonize { body } => state.body_position(body),
        // `Haul` 的**腿别取决于货舱**（有货去 `to`、空舱去 `from`，见 [`haul_step`]），
        // 而这个函数拿不到舰 ⇒ 只能给「待装那一端」。两条真正的执行路径（玩家 / AI）都在
        // 到达 `Haul` 之前就分派给 [`haul_step`] 了，所以这个臂**只为穷尽匹配存在**。
        ShipBehavior::Haul { from, .. } => state.body_position(from),
        ShipBehavior::Idle => [0.0, 0.0],
    }
}

// --- diplomacy --------------------------------------------------------------
