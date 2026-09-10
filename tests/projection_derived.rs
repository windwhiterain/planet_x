//! `--derived` 与 `--index` 是**同一回合的两个读面**，必须给同一个值。
//!
//! 这条不变量值得单独立一个集成测试，因为它跨进程、跨两条代码路径（`--index` 在写出时
//! 用内存里的 `Derived`；`--derived` 读的是 checkpoint 里存下来的那一对），并且是 Python
//! 侧一切 join 的地基：如果两个读面对同一回合各说各话，agent 会照着"另一个世界"的数字
//! 施政——**失败看起来像成功**里最贵的一种。
//!
//! 另外钉住：没有 checkpoint 时 `--derived` 必须**明说**自己是从当前状态重算的（`note`），
//! 而不是默默给出一份 `flow` 为空、看起来像"本回合没有任何产出"的假数据。

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_planet_x"))
}

/// 一个测试专属的临时目录（同一进程内各测试的 pid 相同，所以名字里带 tag）。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("planet_x_derived_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("跑不动 {}: {e}", bin().display()))
}

/// `main.jsonl` 的最后一行（= 最后一回合）。
fn last_main_row(dir: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("main.jsonl")).unwrap();
    let last = text.lines().filter(|l| !l.trim().is_empty()).next_back().unwrap();
    serde_json::from_str(last).unwrap()
}

/// 某张派生表里某个回合的全部行。
fn derived_rows(dir: &std::path::Path, table: &str, round: u64) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join("idx").join(format!("{table}.jsonl"))).unwrap();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|r| r["round"] == serde_json::json!(round))
        .collect()
}

/// 归一化一张「资源 → 数量」映射：缺项（null）与空表等价。
///
/// 投影表的契约是"**永远**给一个对象"（`{}`），这样 Python 侧 `production` 列的 dtype 稳定、
/// 不必为"没产出"写 `isna()` 分支；而 `Derived` 里根本没有这个势力的键（`null`）。
/// 两者语义相同，这个函数就是那道翻译。
fn res_map(v: &serde_json::Value) -> serde_json::Value {
    if v.is_null() {
        serde_json::json!({})
    } else {
        v.clone()
    }
}

/// 治理流的**存在性翻译**（与 `res_map` 同一类）：本回合**没跑治理步骤**的势力
/// （零城势力——`step_governance` 在 `cities.is_empty()` 时 `continue`）在
/// `Derived::flow.governance` 里没有键，而投影表仍旧给它一行，值是引擎自己的约定
/// `total 0.0 / coverage 1.0`（`sim.rs` 的 `governance_total ≈ 0 ⇒ coverage = 1.0`，
/// 也就是「无账可付」，而不是 `GovernanceFlow::default()` 的 0.0「付不起」）。
///
/// 这条差值曾经真的在合并后炸出来过：`flow.jsonl` 说覆盖 0%、同回合的
/// `metrics.factions[].governance_coverage` 说 100%——两个读面各说各话正是这张表
/// 要防的事，所以默认值也必须与引擎同源。
fn gov_of(post_flow: &serde_json::Value, fid: &str) -> (serde_json::Value, serde_json::Value) {
    match post_flow["governance"].get(fid) {
        Some(g) => (g["total"].clone(), g["coverage"].clone()),
        None => (serde_json::json!(0.0), serde_json::json!(1.0)),
    }
}

#[test]
fn derived_matches_the_projection_for_the_same_round() {
    let s = Scratch::new("match");
    let out = s.0.join("out");
    let ckpt = s.0.join("ckpt.ron");
    let (out_s, ckpt_s) = (out.to_str().unwrap(), ckpt.to_str().unwrap());

    // 1) 投影 6 回合，并顺手存一份 checkpoint（应当带上最后一回合的 pre/post，含流量）。
    let st = run(&["--seed", "7", "--round", "6", "--index", out_s, "--save", ckpt_s]);
    assert!(st.status.success(), "--index 失败: {}", String::from_utf8_lossy(&st.stderr));

    // 2) 单点导出同一回合的派生态。
    let st = run(&["--start", ckpt_s, "--derived"]);
    assert!(st.status.success(), "--derived 失败: {}", String::from_utf8_lossy(&st.stderr));
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).expect("--derived 必须输出一行 JSON");
    assert_eq!(v["source"], serde_json::json!("checkpoint"), "有档时必须报 checkpoint 来源");
    assert_eq!(v["round"], serde_json::json!(6));
    assert!(v.get("note").is_none(), "有档时不该有 note（那份派生态是真的）");

    // 3) 主流最后一行的 metrics：必须等于 ckpt 里 post.metrics（逐值相等，不是"差不多"）。
    let main = last_main_row(&out);
    assert_eq!(main["round"], serde_json::json!(6));
    assert_eq!(
        main["metrics"], v["post"]["metrics"],
        "main.jsonl 的 metrics 与 ckpt 的 post.metrics 不是同一个值——两个读面各说各话"
    );

    // 4) flow 表最后回合的每一行：必须等于 ckpt 里 post.flow 的对应项。
    let flow = derived_rows(&out, "flow", 6);
    assert!(!flow.is_empty(), "idx/flow.jsonl 在最后一回合应有行");
    let post_flow = &v["post"]["flow"];
    let mut checked = 0usize;
    let mut governance_ran = 0usize;
    for row in &flow {
        let fid = row["faction_id"].as_str().unwrap();
        let expect_upkeep = &post_flow["upkeep"][fid];
        assert!(
            !expect_upkeep.is_null(),
            "ckpt 的 post.flow 里没有 {fid} 的 upkeep——checkpoint 丢了流量（--index --save 的旧 bug）"
        );
        assert_eq!(&row["upkeep"], expect_upkeep, "{fid} 的 upkeep 两个读面不一致");
        assert_eq!(
            res_map(&row["production"]),
            res_map(&post_flow["faction_production"][fid]),
            "{fid} 的 production 两个读面不一致"
        );
        // 治理同理，只是多一层**存在性**翻译，见 `gov_of` 的说明。
        let (exp_total, exp_cov) = gov_of(post_flow, fid);
        assert_eq!(row["governance_total"], exp_total, "{fid} 的治理总开销两个读面不一致");
        assert_eq!(row["governance_coverage"], exp_cov, "{fid} 的治理覆盖率两个读面不一致");
        if !post_flow["governance"][fid].is_null() {
            governance_ran += 1;
        }
        checked += 1;
    }
    assert!(checked >= 2, "只比对到 {checked} 个势力——守卫太空（至少要有 >=2 个）");
    assert!(
        governance_ran >= 1,
        "这一回合没有任何势力真的跑过治理——那条「存在性翻译」会退化成空转，守卫就白写了"
    );

    // 5) city_flow 同理，且必须真有带产出的行（否则等于没检查）。
    let city_flow = derived_rows(&out, "city_flow", 6);
    let with_prod: Vec<&serde_json::Value> = city_flow
        .iter()
        .filter(|r| r["production"].as_object().map(|o| !o.is_empty()).unwrap_or(false))
        .collect();
    assert!(!with_prod.is_empty(), "最后一回合没有任何带产出的城行");
    for row in with_prod {
        let cid = row["city_id"].as_str().unwrap();
        assert_eq!(
            res_map(&row["production"]),
            res_map(&post_flow["city_production"][cid]),
            "{cid} 的产出两个读面不一致"
        );
    }

    // 6) 控制面表也要在（读面即写面的 tidy 版），并且 join 列真的存在于主流。
    for table in ["control", "scope"] {
        assert!(!derived_rows(&out, table, 6).is_empty(), "idx/{table}.jsonl 在最后一回合应有行");
    }
    assert!(main["faction_ids"].is_array(), "main 行要有 faction_ids（派生表的 join 列）");
}

/// **投影一份 checkpoint 时，起点那一行的流量必须是那一回合的真数**，不是 0。
///
/// 档里的 `post` 就是「产生这个状态的那一回合」的派生态，而 `--start ckpt --round 0 --index`
/// 写出的回合 0 行的 state 正是那个状态。若这里退回 `derived_from_state`，agent 会看到
/// 「全世界零产出、零维护、零治理」——一个数字上自洽、语义上骗人的读面。
#[test]
fn projecting_a_checkpoint_keeps_that_rounds_flow() {
    let s = Scratch::new("seedflow");
    let first = s.0.join("run");
    let ckpt = s.0.join("ckpt.ron");
    let st = run(&["--seed", "7", "--round", "4", "--index", first.to_str().unwrap(), "--save", ckpt.to_str().unwrap()]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let ckpt_s = ckpt.to_str().unwrap();

    // 参考：档里存下来的派生态。
    let st = run(&["--start", ckpt_s, "--derived"]);
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    let round = v["round"].as_u64().unwrap();
    assert_eq!(round, 4);

    // 只用这个 checkpoint 投影（不推进任何回合）。
    let proj = s.0.join("proj");
    let st = run(&["--start", ckpt_s, "--round", "0", "--index", proj.to_str().unwrap()]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));

    let main = last_main_row(&proj);
    assert_eq!(main["round"], serde_json::json!(round));
    assert_eq!(
        main["metrics"], v["post"]["metrics"],
        "起点回合的 metrics 必须是档里那一回合的观测，不是重算（重算会把产出/维护/治理抹成 0）"
    );
    let flow = derived_rows(&proj, "flow", round);
    assert!(!flow.is_empty(), "起点回合应有 flow 行");
    let mut nonzero = 0usize;
    for row in &flow {
        let fid = row["faction_id"].as_str().unwrap();
        assert_eq!(&row["upkeep"], &v["post"]["flow"]["upkeep"][fid], "{fid} 的 upkeep 应为档里的真数");
        assert_eq!(
            res_map(&row["production"]),
            res_map(&v["post"]["flow"]["faction_production"][fid]),
            "{fid} 的 production 应为档里的真数"
        );
        if row["upkeep"].as_f64().unwrap_or(0.0) > 0.0 {
            nonzero += 1;
        }
    }
    assert!(nonzero > 0, "这条守卫要求至少有一个势力的维护费非零，否则等于没检查（零值也能骗过相等断言）");

    // 对照：全新开局的回合 0 确实没有流量（那是初始世界，没有"上一回合"）。
    let fresh = s.0.join("fresh");
    let st = run(&["--seed", "7", "--round", "0", "--index", fresh.to_str().unwrap()]);
    assert!(st.status.success());
    assert!(
        derived_rows(&fresh, "flow", 0).iter().all(|r| r["upkeep"].as_f64().unwrap_or(0.0) == 0.0),
        "全新开局的回合 0 不该有流量"
    );
}

#[test]
fn derived_without_checkpoint_says_it_was_recomputed() {
    let st = run(&["--seed", "7", "--derived"]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    assert_eq!(v["source"], serde_json::json!("state"));
    assert!(v["note"].is_string(), "按当前状态重算时必须给出 note，别让空的 flow 看起来像事实");
    // 没有档就没有本回合流量：这里必须是空的（而不是报错/编数）。
    assert_eq!(v["post"]["flow"]["upkeep"], serde_json::json!({}));
    // 但 metrics 仍然是真的（从当前状态汇总），不该是空壳。
    assert!(v["post"]["metrics"]["factions"].as_object().unwrap().len() >= 2);
}

/// **判定表（`idx/decisions.jsonl`）的契约**：它必须与 `--derived` 里**同一回合存下来的**
/// 判定逐条对得上（跨进程、跨两条代码路径），而且**必须真有东西**。
///
/// 为什么单独钉：这是一张"空白也有意义"的表（`hold` = 这回合 AI 没派活），所以最容易
/// 悄悄退化成一张永远为空的表——那比没有表更坏，它看起来像"AI 这一回合什么也没决定"。
#[test]
fn decisions_table_matches_the_derived_record() {
    let s = Scratch::new("decisions");
    let out = s.0.join("out");
    let ckpt = s.0.join("ckpt.ron");
    let (out_s, ckpt_s) = (out.to_str().unwrap(), ckpt.to_str().unwrap());

    // 跑到有仗打的回合（seed 7：r3 开战）——否则 engage 之类的分支永远走不到。
    let st = run(&["--seed", "7", "--round", "20", "--index", out_s, "--save", ckpt_s]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));

    let st = run(&["--start", ckpt_s, "--derived"]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    let round = v["round"].as_u64().unwrap();
    let dec = &v["post"]["flow"]["decisions"];
    let ships = dec["ships"].as_array().expect("decisions.ships 是数组");
    let retools = dec["retools"].as_array().expect("decisions.retools 是数组");

    let rows = derived_rows(&out, "decisions", round);
    let order_rows: Vec<&serde_json::Value> =
        rows.iter().filter(|r| r["kind"] == serde_json::json!("ship_order")).collect();
    let retool_rows: Vec<&serde_json::Value> =
        rows.iter().filter(|r| r["kind"] == serde_json::json!("retool")).collect();
    assert_eq!(order_rows.len(), ships.len(), "逐舰判定的条数两个读面不一致");
    assert_eq!(retool_rows.len(), retools.len(), "改装判定的条数两个读面不一致");

    const KNOWN: [&str; 7] = ["withdraw", "engage", "colonize", "bombard", "move", "haul", "hold"];
    for d in ships {
        let actor = d["ship"].as_str().unwrap();
        // 一艘舰一回合**最多两行**（先机动、到位后再判一次），所以配对键是
        // (actor, verdict, after_move) 而不是 actor。
        let row = order_rows
            .iter()
            .find(|r| {
                r["actor"] == serde_json::json!(actor)
                    && r["verdict"] == d["verdict"]
                    && r["detail"]["after_move"] == d["after_move"]
            })
            .unwrap_or_else(|| panic!("decisions 表缺 {actor} 的判定行（{d}）"));
        assert_eq!(row["faction_id"], d["faction"], "{actor} 的势力不一致");
        assert_eq!(row["target"], d["target"], "{actor} 的目标不一致");
        // 输入那一半也要对得上——读表的人正是靠它解释"为什么"。
        assert_eq!(row["detail"]["hull_ratio"], d["hull_ratio"], "{actor} 的血量比不一致");
        assert_eq!(row["detail"]["retreat_hull"], d["retreat_hull"], "{actor} 的撤退阈值不一致");
        assert_eq!(row["detail"]["kiting"], d["kiting"], "{actor} 的风筝距离不一致");
        assert_eq!(row["detail"]["enemy_in_range"], d["enemy_in_range"], "{actor} 的敌情不一致");
        assert_eq!(row["detail"]["destination"], d["destination"], "{actor} 的目的地不一致");
        assert_eq!(row["detail"]["order"], d["order"], "{actor} 写回的行为不一致");
        assert!(
            KNOWN.contains(&row["verdict"].as_str().unwrap()),
            "出现了没在 schema 里声明过的判定：{row}"
        );
    }
    for r in retools {
        let city = r["city"].as_str().unwrap();
        let row = retool_rows
            .iter()
            .find(|x| x["actor"] == serde_json::json!(city))
            .unwrap_or_else(|| panic!("decisions 表缺 {city} 的改装行"));
        assert_eq!(row["faction_id"], r["faction"], "{city} 的势力不一致");
        assert_eq!(row["target"], r["to"], "{city} 改装后的舰级不一致");
        assert_eq!(row["detail"]["from"], r["from"], "{city} 改装前的舰级不一致");
        assert_eq!(row["detail"]["building"], r["building"], "{city} 的建筑下标不一致");
    }

    // 防空转：这 20 回合里必须真的发生过接战（否则上面对得再齐也只是空表对空表）。
    let verdicts: std::collections::BTreeSet<&str> =
        order_rows.iter().map(|r| r["verdict"].as_str().unwrap()).collect();
    assert!(
        verdicts.contains("engage"),
        "seed 7 的 20 回合里应当真的出现过接战判定，实际只见到 {verdicts:?}"
    );
    assert!(
        order_rows.len() >= 4,
        "最后一回合的逐舰判定只有 {} 条——守卫太空",
        order_rows.len()
    );
}
