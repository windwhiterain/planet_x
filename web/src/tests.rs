//! WebUI 服务端的单元测试（HTTP 面 / 生命周期）。

use super::*;
use planet_x::config::load_config_from;

/// The workspace config, anchored on this crate's manifest dir so the tests do
/// not depend on the process cwd (cargo runs them with cwd = `web/`).
fn world() -> GameWorld {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../config/game.ron"));
    let config = load_config_from(path).expect("workspace config/game.ron loads");
    let state = world::default_state(&config, 42);
    GameWorld::new(state, config, Prng::new(42))
}

/// 信息树必须是**整份模型**的 dump——没有任何手工挑字段。手工挑字段正是会漂移
/// 的东西：`StateView` 手工列举的字段之外（`ship_name_seq` / `schema_version` /
/// 各实体的全部字段）也必须都在树里。这条测试就是「模型加字段，前端自动看到」
/// 这个不变量的守门人。
#[test]
fn info_roots_are_whole_model_dumps() {
    let w = world();
    let roots = info_roots(&w);
    let names: Vec<&str> = roots.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["state", "pre", "post", "config", "session"]);

    let state = &roots[0].value;
    assert_eq!(
        state,
        &planet_x::json::to_value(&w.state).unwrap(),
        "the `state` root must be the raw State, not a projection"
    );
    for k in [
        "schema_version",
        "ship_name_seq",
        "control",
        "scope",
        "events",
        "chronicle",
    ] {
        assert!(
            state.get(k).is_some(),
            "State field `{k}` missing from the info tree"
        );
    }
    // 实体也要带**全部**字段（不是给地图用的那套拍平视图）。
    let ship = &state["ships"][0];
    for k in ["name", "faction_id", "doctrine", "kiting"] {
        assert!(
            ship.get(k).is_some(),
            "Ship field `{k}` missing from the info tree"
        );
    }

    // 可控 state 也整份在树里（含 RON 特有的元组键 → "城市|建筑id" 字符串键）。
    let ctrl = &state["control"];
    assert!(
        ctrl.is_object(),
        "control must be dumped as a map of faction -> ControllableState"
    );

    for i in [1, 2] {
        // `pre` / `post` 两个根**就是**那份视图本身（同形、拍平）：
        // 观测（含权威的 `faction_power`）与过程量同处一个对象。
        assert!(
            roots[i].value.get("faction_power").is_some(),
            "the round view must carry faction_power"
        );
        assert!(roots[i].value.get("factions").is_some());
        assert!(roots[i].value.get("cities").is_some());
    }

    let config = &roots[3].value;
    for k in [
        "economy",
        "combat",
        "diplomacy",
        "market",
        "governance",
        "mond",
        "balance",
        "ideology",
        "resources",
        "structures",
        "body_kinds",
        "ships",
        "components",
        "buildings",
        "story",
        "name_pool",
    ] {
        assert!(
            config.get(k).is_some(),
            "GameConfig section `{k}` missing from the info tree"
        );
    }

    assert!(roots[4].value.get("rng_state").is_some());
}

/// The info tree must survive a round of simulation (events/chronicle filled
/// in) and stay serializable end-to-end — that is what `/api/state` returns.
/// It must also carry the **real** per-round process quantities, which only `sim::advance`
/// produces (production / upkeep / governance captured while stepping).
#[test]
fn info_tree_carries_real_round_flow_after_advance() {
    let mut w = world();
    assert!(
        w.post
            .factions
            .values()
            .all(|r| r.production.is_empty()),
        "round 0 has no process quantities yet"
    );
    for _ in 0..5 {
        w.pre = sim::view_from_state(&w.state, &w.config);
        w.post = sim::advance(&mut w.state, &w.config, &mut w.rng);
    }
    let view = state_view(&w);
    let json = serde_json::to_value(&view).expect("StateView serializes");
    assert_eq!(json["info"][0]["value"]["round"].as_u64().unwrap(), 5);
    assert!(json["info"][0]["value"]["events"].is_array());
    // 读面**不许**再有手工投影字段：响应的顶层只有写面（control/scope）与整份树。
    // 这条测试是「想再塞一个给前端用的拍平字段」时的守门人——要读什么，从树里取。
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        keys,
        ["control", "info", "scope"],
        "StateView must stay control+scope+info"
    );
    let view = &json["info"][2]["value"];
    // 视图是**一个对象**：`factions` / `cities` 每行都同时带观测与本回合过程量，
    // 所以「过程量真的算过」的判据 = 至少有一个势力的产出/维护/治理不是空的/零。
    let rows = view["factions"].as_object().unwrap();
    assert!(
        rows.values().any(|r| !r["production"].as_object().unwrap().is_empty()),
        "the round's process quantities must be real, not empty"
    );
    assert!(rows.values().any(|r| r["upkeep"].as_f64().unwrap() > 0.0));
    assert!(rows.values().any(|r| r["governance_cost"].as_f64().unwrap() > 0.0));
    assert!(!view["power_share"].as_object().unwrap().is_empty());
}

/// 起始端口空着时，自动模式**必须**原样用它——「`3000` 空着就和从前一样」是这条
/// 特性的全部兼容性承诺，破了它等于偷偷改掉所有人的 URL。
///
/// 「空着」的前提是「问内核要一个空闲端口 → 放手 → 交给 `bind_auto`」，而这两步之间那个
/// 端口可能被**别人**抢走（同一进程里并行的测试、一次出向连接都算）。所以这里**重试**：
/// 被抢走时的症状是 `bind_auto` 返回了**别的**端口，而重试能把它与"实现真的用了起始
/// 端口"区分开。实测在 `cargo test --workspace` 的并行跑里偶发（单独跑 5/5 通过）。
#[tokio::test]
async fn bind_auto_keeps_the_base_port_when_it_is_free() {
    for attempt in 1..=8 {
        // 先问内核要一个肯定空闲的端口，再放手把它交给 `bind_auto`。
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let listener = bind_auto("127.0.0.1", port, 16)
            .await
            .expect("一个刚放手的端口能重绑");
        if listener.local_addr().unwrap().port() == port {
            return;
        }
        assert!(
            attempt < 8,
            "连续 8 个「刚放手的端口」都被抢走了 —— 这不像是巧合"
        );
    }
}

/// 占用时**让位**：向上扫到下一个空闲端口，而不是把「地址已占用」原样抛出去——
/// 这正是「本机已经有 `3000` 在跑」时想要的。也顺带钉住「绝不返回已占端口」。
#[tokio::test]
async fn bind_auto_skips_a_busy_port() {
    let held = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let busy = held.local_addr().unwrap().port();

    let listener = bind_auto("127.0.0.1", busy, 16)
        .await
        .expect("邻居端口总有空的");
    let picked = listener.local_addr().unwrap().port();
    assert_ne!(picked, busy, "自动模式不能把已经被占的端口当成自己的");
    assert!(picked >= busy, "自动模式只向上扫：{picked} < {busy}");
}

/// 环境变量的读法：`0`/`false`/`no`/`off` 是关，别的（含乱写）是开，没设/空回落默认。
#[test]
fn env_flag_and_u64_parsing() {
    assert!(parse_flag(None, true), "没设 = 默认");
    assert!(!parse_flag(None, false));
    assert!(parse_flag(Some(""), true), "空 = 默认");
    assert!(!parse_flag(Some(" 0 "), true));
    assert!(!parse_flag(Some("FALSE"), true));
    assert!(!parse_flag(Some("no"), true));
    assert!(!parse_flag(Some("off"), true));
    assert!(parse_flag(Some("1"), false));
    assert!(parse_flag(Some("yes"), false));

    assert_eq!(parse_u64(None, 500), 500);
    assert_eq!(parse_u64(Some(""), 500), 500);
    assert_eq!(parse_u64(Some(" 0 "), 500), 0, "0 = 关页面立刻退，是合法值");
    assert_eq!(parse_u64(Some("1200"), 500), 1200);
    assert_eq!(parse_u64(Some("banana"), 500), 500);
}

/// 页面登记：陌生 id 不许动任何东西。这是「迟到的注销把当前服务带走」的守门人——
/// 一个来自**上一个**服务的 `bye` 不该让新服务退出。
#[tokio::test]
async fn unknown_tab_id_changes_nothing() {
    let (web, _rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
    assert_eq!(web.close_tab("never-seen"), None);
    assert_eq!(web.tab_count(), 0);
    web.open_tab("a");
    assert_eq!(web.close_tab("bogus"), None);
    assert_eq!(web.tab_count(), 1, "陌生 id 不该让别人的登记消失");
}

/// 最后一个页面关掉才退；还留着一个页面时不许退。
#[tokio::test]
async fn last_tab_leaving_retires_the_server() {
    let (web, mut rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
    web.open_tab("a");
    web.open_tab("b");

    // a 走了，b 还在 → 不许退。
    assert_eq!(web.close_tab("a"), Some(1));
    web.tab_closed();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(rx.try_recv().is_err(), "还有一个页面在看，不该退");

    // b 也走了 → 退，并且带上人话理由。
    assert_eq!(web.close_tab("b"), Some(0));
    web.tab_closed();
    let why = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("最后一个页面关掉必须触发退出")
        .expect("理由要送到");
    assert!(why.contains("页面"), "退出理由要说清是谁让退的：{why}");
}

/// `PLANET_X_WEB_CLOSE_EXIT=0` 时，关页面**不**退（只留启动者租约这条命）。
#[tokio::test]
async fn close_exit_can_be_disabled() {
    let (web, mut rx) = WebCtx::with_close_policy(0, None, false, Duration::ZERO);
    web.open_tab("only");
    assert_eq!(web.close_tab("only"), Some(0));
    web.tab_closed();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(rx.try_recv().is_err(), "关掉 close-exit 后关页面不该退");
}

/// 退出闸门只认第一次：后来的理由不许覆盖第一个（也不许再触发一次）。
#[tokio::test]
async fn exit_gate_fires_once() {
    let (web, rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
    let gate = web.exit_gate();
    assert!(gate.request("第一个理由"));
    assert!(!gate.request("第二个理由"), "第二次不许再触发");
    assert_eq!(rx.await.unwrap(), "第一个理由");
}

/// `/api/ping` 必须如实报出身份：pid / 端口 / 二进制路径 / 页面数。agent 靠它区分
/// 「3001 上那个是不是我刚起的」。
#[tokio::test]
async fn ping_reports_identity() {
    let (web, _rx) = WebCtx::with_close_policy(3013, Some(4242), true, Duration::ZERO);
    web.open_tab("t");
    let Json(id) = ping(AxExtension(web.clone())).await;
    assert_eq!(id.app, "planet_x_web");
    assert_eq!(id.pid, std::process::id());
    assert_eq!(id.port, 3013);
    assert_eq!(id.owner_pid, Some(4242));
    assert_eq!(id.tabs, 1);
    assert!(
        id.exe.ends_with(".exe") || !id.exe.is_empty(),
        "要报出在跑哪个文件"
    );
}

/// 3 条生命周期路由都要注册上（`/api/ping` GET，`/api/tab`、`/api/bye` POST）——
/// 前端 `pagehide` 的注销打不中就等于没有自退。
#[test]
fn lifecycle_routes_are_mounted() {
    let (web, _rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
    let shared: Shared = Arc::new(Mutex::new(world()));
    let app = router(shared, web);
    // `Router` 没有公开的路由表读取接口；用 `Debug` 打印确认这三条真的挂上了。
    let dumped = format!("{app:?}");
    for path in ["/api/ping", "/api/tab", "/api/bye"] {
        assert!(dumped.contains(path), "{path} 没挂上：{dumped}");
    }
}

/// 势力级**默认风格**两片（`default_doctrine` / `default_kiting`）走 web 的写面：
/// 「只写 mode」（前端把归属改成玩家）合法且落地，再写值（前端编辑器里的数）立刻在读面
/// 回显——这就是「势力级两行」的前端往返。最后值**真的生效**：叶 Inherit 的舰改用默认。
#[test]
fn fleet_default_style_rows_round_trip_through_the_web_surface() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let fc_of = |v: &StateView| {
        v.control
            .iter()
            .find(|c| c.faction_id == fid)
            .cloned()
            .unwrap()
    };

    // 开局没有任何人表态：读面**不给**这两行（前端于是补一片 `Inherit` 的叶让行出现）。
    let fc = fc_of(&state_view(&w));
    assert!(
        fc.default_doctrine.is_none() && fc.default_kiting.is_none(),
        "开局不该有默认风格叶"
    );

    // 第一步：前端把这两行的归属改成「玩家」——**只写 mode、不写值**是合法的（值不动），
    // 也不算「写值即接管」。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "default_doctrine": { "mode": "Player" },
            "default_kiting": { "mode": "Player" } }]
    }))
    .expect("前端写的就是这个形状");
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "两条新行必须落地：{:?}", report.skipped);
    assert!(
        report.took_over.is_empty(),
        "只写 mode 不算接管：{:?}",
        report.took_over
    );

    // 第二步：写值（编辑器里的两个数 / 一个数）。读面必须立刻回显——
    // 「点了应用、刷新页面还在」靠的就是这条链。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "default_doctrine": { "temper": 0.4, "lone_wolf": -0.6 },
            "default_kiting": { "kiting": -1.0 } }]
    }))
    .expect("前端写的就是这个形状");
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);

    let fc = fc_of(&state_view(&w));
    let d = fc.default_doctrine.expect("势力级默认风格要在读面里");
    assert_eq!(
        (d.temper, d.lone_wolf, d.mode),
        (Some(0.4), Some(-0.6), Some(ControlMode::Player))
    );
    let k = fc.default_kiting.expect("势力级默认风筝姿态要在读面里");
    assert_eq!((k.kiting, k.mode), (Some(-1.0), Some(ControlMode::Player)));

    // 值真的生效：叶还 Inherit 的舰（开局就是这样，AI 从不写这两片叶）改用舰队默认。
    let sid = w
        .state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .unwrap()
        .name
        .clone();
    let eff = w.state.ship_doctrine(sid.clone());
    assert_eq!(
        (eff.temper, eff.lone_wolf),
        (0.4, -0.6),
        "叶 Inherit + 默认是玩家 ⇒ 取默认值"
    );
    assert_eq!(w.state.ship_kiting(sid.clone()), -1.0);
    assert_eq!(w.state.ship_doctrine_control(sid), ControlMode::Player);
}

/// 前端「点应用」= 把 `/api/state` 的 `control`/`scope` 两段**原样** POST 回 `/api/command`。
/// 这条路径必须**风格中性**：读面给的是「有效值 + 叶片表态」，回传后每艘舰的有效风格与
/// 有效归属逐舰不变。
///
/// 这条测试的靶子是读面里那些「状态中还没有叶片」的行（`ship_doctrine`/`ship_kiting`
/// 对**每艘舰**都有一行，哪怕叶不存在）：原样回传会把有效值写进一片 `Inherit` 的叶——
/// 引擎刻意允许（值不会被采用），但「真的没变」值得被钉住，否则一次误改就会把全舰队的
/// 风格静默改成读面那一刻的快照。
#[test]
fn posting_the_read_surface_back_keeps_effective_style() {
    let mut w = world();
    // 先推几回合，让世界不是开局那一张脸（叶子上有 AI 流水、舰队有增减）。
    for _ in 0..3 {
        w.pre = sim::view_from_state(&w.state, &w.config);
        w.post = sim::advance(&mut w.state, &w.config, &mut w.rng);
    }
    // 让舰队默认风格成为**玩家表态**：这样「叶 Inherit ⇒ 取默认值」这条路径也真的参与进来。
    let fid = w.state.factions[0].name.clone();
    let take: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "default_doctrine": { "temper": 0.5, "lone_wolf": -0.25, "mode": "Player" },
            "default_kiting": { "kiting": -0.8, "mode": "Player" } }]
    }))
    .unwrap();
    assert!(apply_diff(&mut w.state, &w.config, &take).is_clean());

    let snap = |w: &GameWorld| -> Vec<(String, f64, f64, f64, ControlMode, ControlMode)> {
        w.state
            .ships
            .iter()
            .map(|s| {
                let d = w.state.ship_doctrine(s.name.clone());
                (
                    s.name.clone(),
                    d.temper,
                    d.lone_wolf,
                    w.state.ship_kiting(s.name.clone()),
                    w.state.ship_doctrine_control(s.name.clone()),
                    w.state.ship_kiting_control(s.name.clone()),
                )
            })
            .collect()
    };
    let before = snap(&w);

    // 浏览器的那一次 POST：读面（写面模板）原样回传。
    let view = state_view(&w);
    let posted = serde_json::json!({ "control": view.control, "scope": view.scope });
    let req: CommandReq =
        serde_json::from_value(posted).expect("读面必须能被写面接受——前端正是把这两段原样回传的");
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.applied > 0, "回传总得碰到点什么");
    assert_eq!(
        snap(&w),
        before,
        "读面原样回传不许改变任何舰的有效风格 / 归属"
    );
}

/// 前端「只回传差异」靠的就是引擎这条契约（note §8 第 3 条）：补丁写成什么形状，就**只有**
/// 那几个字段被写——别的叶、别的舰一个都不许动。顺带钉住三件反直觉的事：
///
/// * **空 diff 是彻底 no-op**（前端"什么都没改"时点「应用」发的就是它）；
/// * **只改一条轴时另一条轴保留当前有效值**——前端正是靠这个才敢逐轴提交；
/// * **「恢复继承」只撤表态、不动值**：`{mode: Inherit}` 之后叶里那个数还在，而引擎的取值
///   规则是"叶存在就用叶里的值"（`leaf.map(..)` 优先于记录值）⇒ 撤销表态**不会**把这个数
///   放回出厂快照。这条与直觉相反，所以让它由测试写着，而不是留在某人的印象里。
///
/// 为什么值得一条测试：这条路上"多写一个字段"的后果是**静默**的——读面里的风格叶给的是
/// **有效值**，把整行原样写回去就把「没有叶 ⇒ 兜底到出厂记录值」变成「叶钉住这个数」，
/// 而现象只是"改舰队默认对这艘舰没用"，事后极难归因。
#[test]
fn minimal_leaf_diffs_touch_only_what_changed() {
    let mut w = world();
    for _ in 0..3 {
        w.pre = sim::view_from_state(&w.state, &w.config);
        w.post = sim::advance(&mut w.state, &w.config, &mut w.rng);
    }
    // 挑一个**有舰队**的势力（`factions[0]` 可能一艘舰都没有，那样证明不了"别的舰没被动"）。
    let fid = w
        .state
        .factions
        .iter()
        .map(|f| f.name.clone())
        .find(|f| w.state.ships.iter().filter(|s| s.faction_id == *f).count() >= 2)
        .expect("世界里得有个有两艘以上舰的势力");
    let ours: Vec<String> = w
        .state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    assert!(
        ours.len() >= 2,
        "要两艘以上的舰才能证明「别的舰没被动」：{ours:?}"
    );
    let ship = ours[0].clone();

    // 逐舰的「有效风格 + 有效归属」快照：任何一片叶被多写一下，这里就会变。
    let snap = |w: &GameWorld| -> Vec<(String, f64, f64, f64, ControlMode, ControlMode)> {
        w.state
            .ships
            .iter()
            .map(|s| {
                let d = w.state.ship_doctrine(s.name.clone());
                (
                    s.name.clone(),
                    d.temper,
                    d.lone_wolf,
                    w.state.ship_kiting(s.name.clone()),
                    w.state.ship_doctrine_control(s.name.clone()),
                    w.state.ship_kiting_control(s.name.clone()),
                )
            })
            .collect()
    };
    // 前提：这艘舰还没有风格叶、本势力也没有把舰队默认风格设成玩家（下面几条断言依赖它）。
    //
    // ⚠ 本用例要的是「**还没有叶**」这个起点（证明"新建一片叶"这条路径只写该写的字段），
    // 而风格轴的执行者（`autocontrol::style`）现在**每回合都在写风格叶** ⇒ 先把这片势力
    // 名下的风格叶清干净（清掉之后有效值回落到舰上记录值，正是本用例想要的起点）。
    // 这不是"绕过新机制"：这个测试根本不跑模拟，它测的是 `/api/command` 那一层的 diff 形状。
    if let Some(c) = w.state.control.get_mut(&fid) {
        c.ship_doctrine.clear();
        c.ship_kiting.clear();
    }
    assert!(
        w.state
            .control
            .get(&fid)
            .and_then(|c| c.ship_doctrine.get(&ship))
            .is_none(),
        "开局不该有风格叶，否则证明不了「新建」这条路径"
    );
    assert_ne!(
        w.state
            .control
            .get(&fid)
            .and_then(|c| c.default_doctrine.as_ref())
            .map(|d| d.mode),
        Some(ControlMode::Player),
        "前提：舰队默认风格不是玩家表态（否则取值会走默认而不是叶）"
    );
    let before = snap(&w);
    let lone_before = w.state.ship_doctrine(ship.clone()).lone_wolf;

    // ① 空 diff：彻底 no-op（前端没改东西时点「应用」发的就是它）。
    let empty: CommandReq = serde_json::from_value(serde_json::json!({ "control": [] })).unwrap();
    let r = apply_diff(&mut w.state, &w.config, &empty);
    assert_eq!((r.applied, r.skipped.len(), r.took_over.len()), (0, 0, 0));
    assert_eq!(snap(&w), before, "空 diff 不该动任何东西");

    // ② 逐轴提交：只写 temper。缺省的那条轴保留**当前有效值**，且这片叶因「写了值没写 mode」
    //    被接管；别的舰、这条舰的另一条轴一律不动。
    let one: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "ship_doctrine": [{ "ship": ship, "temper": 0.33 }] }]
    }))
    .unwrap();
    let r = apply_diff(&mut w.state, &w.config, &one);
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(
        r.took_over.len(),
        1,
        "写值即接管必须留一条回执：{:?}",
        r.took_over
    );
    let leaf = w.state.control[&fid].ship_doctrine[&ship].clone();
    assert_eq!(
        (leaf.value.temper, leaf.value.lone_wolf),
        (0.33, lone_before),
        "缺省的那条轴必须保留现值"
    );
    assert_eq!(leaf.mode, ControlMode::Player);

    let after = snap(&w);
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0);
        if a.0 != ship {
            assert_eq!(b, a, "只改一艘舰的叶，别的舰不该被动");
        }
    }
    let edited = |v: &Vec<(String, f64, f64, f64, ControlMode, ControlMode)>| {
        v.iter().find(|x| x.0 == ship).cloned().unwrap()
    };
    assert_eq!(
        edited(&after).2,
        lone_before,
        "没写的那条轴的有效值也不许变"
    );
    assert_eq!(edited(&after).3, edited(&before).3, "风筝轴一个字都不该动");
    assert_eq!(edited(&after).4, ControlMode::Player, "接管之后归属是玩家");

    // ③ 「恢复继承」（`{mode: Inherit}`）：只撤表态、值不动——而且引擎的取值规则让叶里那个数
    //    **继续生效**。补丁接口只能新建/改写叶、删不掉叶，所以"收回出厂快照"今天做不到。
    let back: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "ship_doctrine": [{ "ship": ship, "mode": "Inherit" }] }]
    }))
    .unwrap();
    let r = apply_diff(&mut w.state, &w.config, &back);
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert!(
        r.took_over.is_empty(),
        "只写 mode 不是接管：{:?}",
        r.took_over
    );
    let leaf = &w.state.control[&fid].ship_doctrine[&ship];
    assert_eq!(leaf.mode, ControlMode::Inherit);
    assert_eq!(
        (leaf.value.temper, leaf.value.lone_wolf),
        (0.33, lone_before),
        "「恢复继承」的契约是值不动"
    );
    assert_eq!(
        w.state.ship_doctrine(ship.clone()).temper,
        0.33,
        "叶存在就用叶里的值，哪怕它说 Inherit"
    );
    assert_eq!(
        w.state.ship_doctrine_control(ship.clone()),
        edited(&before).4,
        "收回表态后归属回到链上（与最初一致）"
    );
}

/// 「恢复出厂值」（**删叶**）走 web 的写面：前端那个按钮发的就是
/// `{"ship": …, "remove": true}`。它与「恢复继承」（只写 mode）**不是**一回事——
/// 叶只要还在，引擎就优先用叶里的值，所以只有删掉它才能回到出厂快照。
#[test]
fn removing_a_ship_style_leaf_returns_the_factory_record() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let ship = w
        .state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("这个势力得有舰");
    // 出厂记录值给成非零（`config/*.ron` 从没填过风格，开局是 {0,0}，那样分不出
    // "回到出厂值"和"钉在 0"）。
    for s in w.state.ships.iter_mut().filter(|s| s.faction_id == fid) {
        s.doctrine = planet_x::model::ShipDoctrine {
            temper: 0.71,
            lone_wolf: -0.2,
        };
    }
    let record = w.state.ship(&ship).unwrap().doctrine;

    // ① 先写一片叶（两轴一起给，避免 `partial_doctrine_leaf`）。
    let take: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "ship_doctrine": [{ "ship": ship, "temper": -1.0, "lone_wolf": 0.5 }] }]
    }))
    .unwrap();
    assert!(apply_diff(&mut w.state, &w.config, &take).is_clean());
    assert_eq!(w.state.ship_doctrine(ship.clone()).temper, -1.0);

    // ② 前端那个按钮的补丁：只带身份键 + `remove`。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "ship_doctrine": [{ "ship": ship, "remove": true }] }]
    }))
    .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);
    assert_eq!(
        report.removed.len(),
        1,
        "删叶要有回执：{:?}",
        report.removed
    );
    assert_eq!(
        w.state.ship_doctrine(ship.clone()),
        record,
        "删叶之后必须回到出厂记录值"
    );
    assert!(
        w.state
            .control
            .get(&fid)
            .and_then(|c| c.ship_doctrine.get(&ship))
            .is_none(),
        "这片叶必须真的没了（前端「当前跟随」那行会立刻改口）"
    );
    // 读面仍然给这艘舰一行（值 = 有效值 = 出厂值）——前端不必为"叶不存在"特判。
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let row = fc
        .ship_doctrine
        .into_iter()
        .find(|e| e.ship == ship)
        .expect("每艘舰一行");
    assert_eq!(
        (row.temper, row.lone_wolf),
        (record.temper, record.lone_wolf)
    );
    assert_eq!(row.mode, ControlMode::Inherit);
}

/// 第三条风格轴（**角色**：运输舰↔战舰）在 web 的读写两面上走通：读面每艘舰一行
/// （`ship_freighter`，值 = 有效值）、写面能定角色（写值即接管 ⇒ 自动控制不再定编这艘舰）、
/// `remove` 能删掉这片叶把它**交回自动定编**（这与另两条风格轴上"删叶"的含义不同）。
#[test]
fn the_role_axis_round_trips_through_the_web_surface() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let ship = w
        .state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("这个势力得有舰");

    // ① 读面：每艘舰都有一行角色（值 = 有效值，开局就是出厂记录值）。
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let row = fc
        .ship_freighter
        .iter()
        .find(|e| e.ship == ship)
        .expect("每艘舰一行角色");
    assert_eq!(row.freighter, w.state.ship(&ship).unwrap().freighter);

    // ② 写面：把一艘舰钉成运输舰（写值即接管 ⇒ 归属变 Player，AI 定编从此不碰它）。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "ship_freighter": [{ "ship": ship, "freighter": true }] }]
    }))
    .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);
    assert!(w.state.ship_freighter(ship.clone()));
    assert_eq!(
        w.state.ship_freighter_control(ship.clone()),
        ControlMode::Player
    );

    // ③ 前端那个「恢复出厂值」按钮发的补丁：只带身份键 + `remove`。
    let record = w.state.ship(&ship).unwrap().freighter;
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "ship_freighter": [{ "ship": ship, "remove": true }] }]
    }))
    .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);
    assert_eq!(
        report.removed.len(),
        1,
        "删叶要有回执：{:?}",
        report.removed
    );
    assert_eq!(
        w.state.ship_freighter(ship.clone()),
        record,
        "删叶之后回到出厂记录值"
    );
    assert!(
        w.state
            .control
            .get(&fid)
            .and_then(|c| c.ship_freighter.get(&ship))
            .is_none(),
        "这片叶必须真的没了"
    );
    assert_ne!(
        w.state.ship_freighter_control(ship.clone()),
        ControlMode::Player,
        "删叶 = 交回自动定编（而不是「锁成某个值」）"
    );

    // ④ 势力级默认角色叶（`Option`：有叶才有一行）也走读写两面。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "default_freighter": { "freighter": true, "mode": "Player" } }]
    }))
    .unwrap();
    assert!(apply_diff(&mut w.state, &w.config, &req).is_clean());
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let d = fc.default_freighter.expect("势力级默认角色叶要在读面里");
    assert_eq!(
        (d.freighter, d.mode),
        (Some(true), Some(ControlMode::Player))
    );
}

/// **指令行（`ship_orders`）在 web 读面上「每舰一行」**——包括**没有叶**的舰。
///
/// 前端那棵树是**从指令行长出来**的（`app.js::buildTree` 遍历 `fc.ship_orders`，一艘舰的
/// 风格 / 角色两行是它的子节点），所以以前「叶被删掉」⇒ 这艘舰**整行从控制树里消失**，
/// 玩家连它的风格都没法再单独设归属（`control-live-layers.md` §10.5 记的缺口）。
///
/// 这条同时钉住前端要区分的那两件事：`behavior`（**有效值**，链上没人说话 = `null`）
/// 与 `mode`（**叶自己的表态**，没有叶 = `Inherit`）。
#[test]
fn the_order_read_face_lists_ships_without_a_leaf() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let ours: Vec<String> = w
        .state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    assert!(ours.len() >= 2, "这个势力得有两艘以上舰");
    let vanished = ours[0].clone();

    // 把一艘舰的指令叶删掉 —— 正是「恢复出厂值」（`remove: true`）之后的状态。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "ship_orders": [{ "ship": vanished, "remove": true }] }]
    }))
    .unwrap();
    assert!(apply_diff(&mut w.state, &w.config, &req).is_clean());

    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let rows: Vec<String> = fc.ship_orders.iter().map(|e| e.ship.clone()).collect();
    assert_eq!(
        rows, ours,
        "指令读面必须每舰一行（含叶被删掉的舰），顺序同 `state.ships`"
    );

    let row = fc.ship_orders.iter().find(|e| e.ship == vanished).unwrap();
    assert_eq!(
        row.behavior, None,
        "链上没人说话 ⇒ `null`（前端据此显示「无人表态」）"
    );
    assert_eq!(row.mode, ControlMode::Inherit, "没有叶 ⇒ 这一层没有说话");
    for other in fc.ship_orders.iter().filter(|e| e.ship != vanished) {
        assert!(
            other.behavior.is_some(),
            "「{}」的叶还在 ⇒ 有效值是一个真行为",
            other.ship
        );
    }

    // 前端「只回传差异」的载荷（身份键 + 只改过的字段）：给这艘没有叶的舰设归属必须落地。
    let req: CommandReq =
        serde_json::from_value(serde_json::json!({ "control": [{ "faction_id": fid,
            "ship_orders": [{ "ship": vanished, "mode": "Player" }] }] }))
        .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let row = fc.ship_orders.iter().find(|e| e.ship == vanished).unwrap();
    assert_eq!(
        row.mode,
        ControlMode::Player,
        "叶被建出来了（只写表态不建叶的规则只管 Inherit）"
    );
    assert!(
        row.behavior.is_some(),
        "建叶时那份值就是当时的有效值兜底（`Idle`）"
    );
}

/// **设计图**在 web 的读写两面上走通：读面给出图库（`blueprints`，含引擎算的 `ship_count`）、
/// 写面能建图 / 改图 / 删图，建造区的指针（`buildings[].blueprint`）能挂上、能拆掉。
///
/// 这一条同时钉住 `app.js` 的建造区编辑器依赖的那两个事实：**图库来自 control 读面**、
/// **指针通过 `buildings` 结构补丁写**（`null` = 拆掉，不是缺席）。
#[test]
fn the_blueprint_library_round_trips_through_the_web_surface() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    // 找一座本势力的建造区（web 的建造区编辑器就在这一行上加「设计图」下拉）。
    let (cid, bid, ship_type) = w
        .state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .find_map(|c| {
            c.buildings.iter().find(|b| b.is_shipyard()).map(|b| {
                (
                    c.name.clone(),
                    b.id,
                    b.ship_type.clone().unwrap_or_default(),
                )
            })
        })
        .expect("这个势力得有建造区");

    // ① 开局：图库是空的（用户裁决 Q8：不预置标准图）。
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    assert!(fc.blueprints.is_empty(), "开局不该有任何设计图");

    // ② 写面：建一张**舰级对得上**的图，并把建造区指过去（同一份 diff：一次成功）。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "blueprints": [{"name": "重甲护卫", "class": ship_type,
                            "components": ["kinetic", "ion_drive"], "mode": "Player"}],
            "buildings": [{"city": cid, "building": bid, "blueprint": "重甲护卫"}] }]
    }))
    .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);

    // ③ 读面：图库一行，选装**全量**给出（否则回传时会静默清空），指针在建造区上。
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    let row = fc
        .blueprints
        .iter()
        .find(|b| b.name == "重甲护卫")
        .expect("读面要给出图库");
    assert_eq!(row.class, ship_type);
    assert_eq!(
        row.components,
        vec!["kinetic".to_string(), "ion_drive".to_string()]
    );
    assert_eq!(row.mode, ControlMode::Player);
    assert_eq!(row.ship_count, 0, "还没造过 ⇒ 0（派生量，现算）");
    assert!(
        !row.launch_waiting,
        "派生的「买不起 ⇒ 未下水」标记：刚建的图没人在等钱"
    );
    assert_eq!(
        w.state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .blueprint
            .as_deref(),
        Some("重甲护卫")
    );

    // ④ 拆指针（前端「（无：自动选装）」那一格发的就是 `null`）⇒ 回到自动选装。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "buildings": [
            {"city": cid, "building": bid, "blueprint": null}] }]
    }))
    .unwrap();
    assert!(apply_diff(&mut w.state, &w.config, &req).is_clean());
    assert_eq!(
        w.state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .blueprint,
        None,
        "`null` 拆掉指针"
    );

    // ⑤ 删整张图（`remove`）：回执里点名到叶，图库里就没了。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid, "blueprints": [{"name": "重甲护卫", "remove": true}] }]
    }))
    .unwrap();
    let report = apply_diff(&mut w.state, &w.config, &req);
    assert!(report.is_clean(), "{:?}", report.skipped);
    assert_eq!(report.removed.len(), 1, "{:?}", report.removed);
    let fc = state_view(&w)
        .control
        .into_iter()
        .find(|c| c.faction_id == fid)
        .unwrap();
    assert!(fc.blueprints.is_empty(), "图被删掉了");
}

/// **图库读面必须带上 `launch_waiting`（Q4(b) 的可见标记）**，而且它不该改变写面：
/// 建造区那一行要能显示「买不起 ⇒ 未下水」，而这条标记的**唯一真值**在引擎里
/// （与投影 `blueprints.launch_waiting` 列同一个函数）——前端不重算它，只显示。
///
/// 这里把判据造全：玩家归属的图 + 带选装 + 建造区挂着它 + 该舰级进度**已经攒够**
/// + 组件**买不起** ⇒ `launch_waiting == true`；钱够了 ⇒ 立刻回 false（它是"此刻"的
/// 派生量，不是新状态）。同时钉住：整面模板回传（含这些只读列）**不许炸写面**。
#[test]
fn the_blueprint_read_face_carries_launch_waiting_and_still_round_trips() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let (cid, bid, ship_type) = first_shipyard(&w, &fid);

    // 建图（玩家归属 + 两件选装）+ 把建造区指过去。
    let req: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "blueprints": [{"name": "等钱的图", "class": ship_type,
                            "components": ["kinetic", "ion_drive"], "mode": "Player"}],
            "buildings": [{"city": cid, "building": bid, "blueprint": "等钱的图"}] }]
    }))
    .unwrap();
    let view = apply_command(&mut w, &req);
    assert!(
        view.report.as_ref().unwrap().is_clean(),
        "{:?}",
        view.report.as_ref().unwrap().skipped
    );
    let waiting = |w: &GameWorld| -> bool {
        state_view(w)
            .control
            .into_iter()
            .find(|c| c.faction_id == fid)
            .unwrap()
            .blueprints
            .into_iter()
            .find(|b| b.name == "等钱的图")
            .expect("读面要给出这张图")
            .launch_waiting
    };
    assert!(!waiting(&w), "进度还是 0 ⇒ 没有人在等钱");

    // 造出「进度攒够却没下水」：该城该舰级的进度写满 `build_points`，库存清零（买不起）。
    let bp = w.config.ship_spec(&ship_type).build_points;
    w.state
        .city_mut(&cid)
        .unwrap()
        .ship_progress
        .insert(ship_type.clone(), bp);
    zero_resources(&mut w, &fid);
    assert!(
        waiting(&w),
        "进度满 + 买不起 ⇒ launch_waiting 必须为真（界面就靠它显示「买不起 ⇒ 未下水」）"
    );

    // 整面模板回传（带着 `ship_count` / `launch_waiting` 两个只读派生列）：
    // **不许**被 `deny_unknown_fields` 判非法（读面即写面）。
    let face = state_view(&w);
    let posted = serde_json::json!({ "control": face.control, "scope": face.scope });
    let req: CommandReq = serde_json::from_value(posted).expect("读面必须能被写面收下");
    assert!(
        apply_command(&mut w, &req).report.unwrap().is_clean(),
        "模板回传不该丢叶"
    );

    // 钱够了 ⇒ 下一回合真的下水，进度被扣掉 ⇒ 标记随之消失。
    // ⚠ 这条派生列的判据是「进度满 **且** 没下水」（不是"直接检查库存"）：买得起之后
    // 出厂循环会把这艘舰放出来、进度减一次 `build_points`，于是标记自然回 false。
    // 所以这里模拟那次扣减，而不是只改库存——否则测的就是一个不存在的语义。
    for (_, v) in w.state.faction_mut(&fid).unwrap().resources.iter_mut() {
        *v = 1e9;
    }
    assert!(
        waiting(&w),
        "只补钱、没下水 ⇒ 进度仍然满着（标记照实说：还没下水）"
    );
    w.state
        .city_mut(&cid)
        .unwrap()
        .ship_progress
        .remove(&ship_type);
    assert!(!waiting(&w), "下水之后进度归零 ⇒ 不再等钱");
}

/// **`POST /api/command` 必须把引擎的回执回给页面**：界面能新建/改/删设计图之后，
/// 「你写的补丁被守卫拒了」是正常会发生的事（`blueprint_class_mismatch` 等），而
/// `--apply` 的语义是"只触碰 diff 里出现的叶片"⇒ 静默丢掉与成功落地在响应上一样，
/// 那正是「失败看起来像成功」。这条测试钉住三件事：
/// 1. 被拒的原因码**从响应里拿得到**（不是只写进日志）；
/// 2. `/api/state` 那种"没跑过 diff"的响应**不带**这个键（老形状逐字节不变）；
/// 3. 被拒之后状态**一个字节不动**。
#[test]
fn a_rejected_blueprint_patch_comes_back_in_the_command_report() {
    let mut w = world();
    let fid = w.state.factions[0].name.clone();
    let (cid, bid, ship_type) = first_shipyard(&w, &fid);

    // `/api/state` 那条路（`state_view`）：没有回执键（`skip_serializing_if`）。
    let plain = serde_json::to_value(state_view(&w)).unwrap();
    assert!(
        plain.get("report").is_none(),
        "只有跑过 diff 的响应才带 report"
    );

    // 先建一张**舰级对得上**的图并挂上指针（合法）。
    let ok: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "blueprints": [{"name": "被拒的图", "class": ship_type, "components": ["kinetic"], "mode": "Player"}],
            "buildings": [{"city": cid, "building": bid, "blueprint": "被拒的图"}] }]
    }))
    .unwrap();
    assert!(apply_command(&mut w, &ok).report.unwrap().is_clean());

    // 再把图的舰级改成**另一个**级：建造区还挂着它 ⇒ 口径 A 的守卫必须拒。
    let other = w
        .config
        .ships
        .keys()
        .find(|k| **k != ship_type)
        .cloned()
        .expect("至少有两个舰级");
    let bad: CommandReq = serde_json::from_value(serde_json::json!({
        "control": [{ "faction_id": fid,
            "blueprints": [{"name": "被拒的图", "class": other}] }]
    }))
    .unwrap();
    let view = apply_command(&mut w, &bad);
    let json = serde_json::to_value(&view).unwrap();
    let skipped = json["report"]["skipped"]
        .as_array()
        .expect("回执必须带着丢弃清单");
    assert_eq!(skipped.len(), 1, "{json}");
    assert_eq!(skipped[0]["code"], "blueprint_class_mismatch");
    // reason 是人读的一句话，且指出**接下来怎么办**（"要么…要么…"）。
    let reason = skipped[0]["reason"].as_str().unwrap();
    assert!(
        reason.contains("要么"),
        "reason 要给出出路（界面照实显示它）：{reason}"
    );
    assert_eq!(
        w.state.control[&fid].blueprints["被拒的图"].value.class, ship_type,
        "被拒 ⇒ 状态一个字节不动"
    );
}

/// 本势力的第一个建造区（城名、建筑下标、该区舰级）——两条设计图测试都要它。
fn first_shipyard(w: &GameWorld, fid: &str) -> (String, u32, String) {
    w.state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .find_map(|c| {
            c.buildings.iter().find(|b| b.is_shipyard()).map(|b| {
                (
                    c.name.clone(),
                    b.id,
                    b.ship_type.clone().unwrap_or_default(),
                )
            })
        })
        .expect("这个势力得有建造区")
}

/// 把某个势力的库存全部清零（用来制造「进度满却买不起」）。
fn zero_resources(w: &mut GameWorld, fid: &str) {
    for (_, v) in w.state.faction_mut(fid).unwrap().resources.iter_mut() {
        *v = 0.0;
    }
}
