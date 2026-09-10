//! MOND 异常导航：非 master 命不中深处目标（但**没有进不去的目标**）、`route_depth` 度量、以及那条只打印的探针。

use super::*;

/// MOND 引力异常：落入异常区（深空）时，未掌握 MOND 修正引力的势力在导航上产生
/// 切向偏移（指令坐标与实际坐标分离），而掌握它的 cult 指哪打哪。
///
/// 偏移幅度是**伪随机**的（`roll`）：非 master 这一回合偏多少由尝试决定，
/// 但**永远存在蒙对的一次**（`roll = 0` 即精确命中）——所以深处是「难」而不是「不可能」。
#[test]
fn mond_drift_misses_in_anomaly_but_masters_are_exact() {
    let (config, _state) = fresh_world(42);
    let dest = [60.0, 0.0]; // 距太阳 60 AU，深入柯伊伯异常区。
    // 非 MOND 势力（中国=3）：这一回合的目标被切向偏移，无法精确到达。
    let d = mond_drift(&config, 0.0, dest, 0.5);
    assert!(
        (d[0] - dest[0]).abs() > 1e-6 || (d[1] - dest[1]).abs() > 1e-6,
        "a non-master ship must drift inside the anomaly, got {d:?}"
    );
    // 但偏移是可变的：偏得少的一次就几乎命中（这是「多试几个回合」的根据）。
    let lucky = mond_drift(&config, 0.0, dest, 0.0);
    assert_eq!(
        lucky, dest,
        "roll=0 的一次尝试必须指哪打哪——不存在永远进不去的目标"
    );
    let worse = mond_drift(&config, 0.0, dest, 1.0);
    assert!(
        dist(worse, dest) > dist(d, dest),
        "roll 越大偏得越远（幅度单调），got {worse:?}"
    );
    // MOND 势力（行星X崇拜教=8）：掌握修正引力，无偏移、指哪打哪。
    for roll in [0.0, 0.5, 1.0] {
        let m = mond_drift(&config, 1.0, dest, roll);
        assert_eq!(
            m, dest,
            "a MOND master must compute the destination exactly"
        );
    }
}

/// **运输能力的地基（机制不变量）**：非 master 舰船抵达某天体的**成功率**是算出来的，
/// 而**不是**一条「能/不能」的硬线。
///
/// `move_toward` 把目标交给 [`mond_drift`] 切向平移 `roll × (r − radius) × drift_per_au`，
/// 舰船只在距（被平移后的）目标 `arrival_eps` 内停泊 ⇒ 它离真实天体的距离就是那个偏移量。
/// 偏移幅度按 `roll ∈ [0,1)` **伪随机**取，而**每回合是一次新的尝试**（[`nav_roll`]），于是
///
/// ```text
/// 每次尝试的成功率  p = min(1, arrival_eps / (depth × drift_per_au)) = min(1, 2/(r − 28))
/// ```
///
/// 三条不变量（用户裁决：**MOND 再强也总有成功几率，只是要多试几个回合**）：
/// 1. `p > 0` 对**任意有限深度**都成立（`roll = 0` 的一次尝试必然命中）——没有绝对不可达；
/// 2. `p` 随深度**单调不增**（越深越难）；
/// 3. `p = 1` 的门槛仍是 `radius + arrival_eps/drift_per_au = 30 AU`——带内近处**一次到位**。
///
/// 实测（期望尝试回合数 = `1/p`）：伊克西翁 p≈0.92→1.1 回合、妊神星 0.29→3.5、
/// 创神星 0.20→5.0、阋神星 0.20→5.0。守卫它就是守住「圣所**难打但打得下来**」这个形状：
/// 调 `radius` / `drift_per_au` / `arrival_eps` 里任何一个都会整张表平移。
#[test]
fn mond_depth_only_costs_attempts_never_makes_it_impossible() {
    let (config, state) = fresh_world(42);
    let m = &config.mond;
    let eps = config.combat.arrival_eps;

    // 1) 任意有限深度都有成功几率——`roll = 0` 的一次尝试必然指哪打哪。
    for depth in [0.5, 2.0, 10.0, 72.0, 10_000.0] {
        let dest = [0.0, m.radius + depth];
        assert_eq!(
            mond_drift(&config, 0.0, dest, 0.0),
            dest,
            "深度 {depth} 也必须存在「蒙对」的一次尝试（roll=0）"
        );
        assert!(
            mond_arrival_chance(&config, depth, 0.0) > 0.0,
            "深度 {depth} 的成功率必须严格大于 0——MOND 再强也不能让目标变成进不去"
        );
    }
    // 连「MOND 强到离谱」也不行：配置随便调，几率只会变小、不会归零。
    let mut harsh = load_config();
    harsh.mond.drift_per_au = 50.0;
    let harsh_p = mond_arrival_chance(&harsh, 2.0, 0.0);
    assert!(harsh_p > 0.0, "drift_per_au=50 时成功率仍然 > 0");
    assert!(harsh_p < 0.01, "…但小到要试上百个回合（实测 p={harsh_p}）");

    // 2) 成功率随深度单调不增。
    let mut prev = 1.0;
    for i in 0..300 {
        let depth = i as f64 * 0.5;
        let p = mond_arrival_chance(&config, depth, 0.0);
        assert!(
            p <= prev + 1e-12,
            "深度 {depth} AU 的成功率不该比更浅处高（{p} > {prev}）"
        );
        prev = p;
    }
    // 3) 30 AU 以内一次到位：门槛 = radius + arrival_eps/drift_per_au。
    let exact = eps / m.drift_per_au;
    assert!(
        (mond_arrival_chance(&config, exact, 0.0) - 1.0).abs() < 1e-9,
        "门槛深度 {exact} AU 处应一次到位"
    );
    assert!(
        mond_arrival_chance(&config, exact + 1.0, 0.0) < 1.0,
        "门槛往外一点就不再是一次到位"
    );

    // 4) 真实天体的表：带内近处一次到位；深处要试几次，但**都试得到**。
    let table: Vec<(String, f64)> = state
        .bodies
        .iter()
        .filter_map(|b| {
            let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
            (r > m.radius).then(|| {
                (
                    b.name.clone(),
                    mond_arrival_chance(&config, r - m.radius, 0.0),
                )
            })
        })
        .collect();
    let chance = |n: &str| {
        table
            .iter()
            .find(|(name, _)| name == n)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| panic!("{n} 应在带内"))
    };
    for shallow in ["海王星", "冥王星", "卡戎"] {
        assert!(
            chance(shallow) >= 1.0 - 1e-9,
            "{shallow} 在 30 AU 以内，应**一次到位**（实测 p={}）",
            chance(shallow)
        );
    }
    // 圣所：难，但一个月内基本能到（期望 ≈1.1 回合）——「堡垒」是**拖时间**，不是绝对挡驾。
    let ik = chance("伊克西翁");
    assert!(
        (0.8..1.0).contains(&ik),
        "伊克西翁成功率应在 0.8–1.0（实测 {ik:.3}，期望 {:.1} 回合）",
        1.0 / ik
    );
    // 柯伊伯矿：要试几次，但**有得试**——这正是承包定价与「超期掉信誉」的基础。
    for deep in ["妊神星", "创神星", "阋神星"] {
        let p = chance(deep);
        assert!(
            (0.05..0.5).contains(&p),
            "{deep} 应是「要试几次但试得到」（实测 p={p:.3}，期望 {:.1} 回合）",
            1.0 / p
        );
    }
    assert!(
        chance("伊克西翁") > chance("妊神星") && chance("妊神星") > chance("创神星"),
        "越深越难（成功率必须随深度递降），实测 伊克西翁 {:.3} / 妊神星 {:.3} / 创神星 {:.3}",
        chance("伊克西翁"),
        chance("妊神星"),
        chance("创神星")
    );

    // 5) 形状旋钮（`drift_shape`）：调小 = 偏移偏向大值 = 更深，但**永远 > 0**。
    let deep = 10.16; // 创神星的深度
    let mut skew = load_config();
    skew.mond.drift_shape = 0.5;
    let mut easy = load_config();
    easy.mond.drift_shape = 2.0;
    let (p_skew, p_uni, p_easy) = (
        mond_arrival_chance(&skew, deep, 0.0),
        mond_arrival_chance(&config, deep, 0.0),
        mond_arrival_chance(&easy, deep, 0.0),
    );
    assert!(
        p_skew < p_uni && p_uni < p_easy,
        "shape 越小越难（实测 skew(0.5)={p_skew:.3} < 均匀={p_uni:.3} < easy(2.0)={p_easy:.3}）"
    );
    assert!(
        p_skew > 0.0,
        "旋钮怎么调都不能把深处变成「进不去」——这是不可破坏的性质"
    );
}

/// 探针（`cargo test --lib probe_mond_attempts -- --ignored --nocapture`）：
/// 用**真实的** [`nav_roll`] 逐回合实测「深处目标要试几个回合」——闭式 `p` 是「单次尝试
/// 的命中率」，这里量的是它**在真实伪随机序列上**的表现（首次命中的回合数、1000 回合里的
/// 命中次数），并且验证**没有任何天体是 0 命中**（= 不存在进不去的目标）。
#[test]
#[ignore]
fn probe_mond_attempts() {
    let (config, state) = fresh_world(42);
    let m = &config.mond;
    let eps = config.combat.arrival_eps;
    println!("== MOND 导航尝试实测（ship=朝圣者, fid=中国；闭式 p = eps/(depth×drift)）==");
    println!(
        "  {:<8} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "天体", "深度AU", "闭式p", "闭式期望", "首次命中", "1000次命中", "命中率"
    );
    for b in &state.bodies {
        let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
        if r <= m.radius {
            continue;
        }
        let depth = r - m.radius;
        let p = mond_arrival_chance(&config, depth, 0.0);
        let mut first: Option<u32> = None;
        let mut hits = 0u32;
        let n = 1000u32;
        for round in 0..n {
            let roll = nav_roll("中国", "朝圣者", round);
            if dist(mond_drift(&config, 0.0, b.position, roll), b.position) <= eps {
                hits += 1;
                first.get_or_insert(round + 1);
            }
        }
        println!(
            "  {:<8} {:>7.2} {:>7.3} {:>9.1} {:>9} {:>9} {:>9.3}",
            b.name,
            depth,
            p,
            if p > 0.0 { 1.0 / p } else { f64::INFINITY },
            first
                .map(|f| f.to_string())
                .unwrap_or_else(|| "从未".into()),
            hits,
            hits as f64 / n as f64
        );
        assert!(
            hits > 0,
            "{} 必须至少命中一次——不存在永远进不去的目标",
            b.name
        );
    }
}

/// **贸易路线的引力异常浸入深度**（M6）：两端都在带外 = 0（普通航线）；
/// 一端在带内、一端在外 = 远端深度（要穿过去）；两端都在带内 = 较浅那端深度。
/// 它是运费倍率与丢货率的唯一驱动量，所以必须有确定的语义。
#[test]
fn route_depth_measures_mond_immersion() {
    let (config, _state) = fresh_world(42);
    let r = config.mond.radius;
    let inside = [r - 5.0, 0.0];
    let shallow = [r + 2.0, 0.0];
    let deep = [r + 10.0, 0.0];
    assert_eq!(
        route_depth(&config, inside, [1.0, 0.0]),
        0.0,
        "两端都在异常带外的航线没有 MOND 代价"
    );
    assert!(
        (route_depth(&config, inside, deep) - 10.0).abs() < 1e-9,
        "一端在带内、一端在 10 AU 深 → 要穿到 10 AU 深"
    );
    assert!(
        (route_depth(&config, shallow, deep) - 2.0).abs() < 1e-9,
        "两端都在带内 → 按较浅那端算（2 AU）"
    );
    // 确定性：交换两端不改变结果（路线是双向的）。
    assert_eq!(
        route_depth(&config, inside, deep),
        route_depth(&config, deep, inside)
    );
}
