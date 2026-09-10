//! `--derived` 与 `--index` 是**同一回合的两个读面**，必须给同一个值。
//!
//! 这条不变量值得单独立一个集成测试，因为它跨进程、跨两条代码路径（`--index` 在写出时
//! 用内存里的 `Derived`；`--derived` 读的是 checkpoint 里存下来的那一对），并且是 Python
//! 侧一切 join 的地基：如果两个读面对同一回合各说各话，agent 会照着"另一个世界"的数字
//! 施政——**失败看起来像成功**里最贵的一种。
//!
//! 另外钉住：没有 checkpoint 时 `--derived` 必须**明说**自己是从当前状态重算的（`note`），
//! 而不是默默给出一份 `flow` 为空、看起来像"本回合没有任何产出"的假数据。
//!
//! ⚠ **这条不变量不适用于 `idx/blueprints.jsonl`**（舰船设计图库）：它**不在** `Derived` 里，
//! 而是**状态**的纯函数（每回合从 `state.control[*].blueprints` 现算），所以没有「两个读面
//! 各说各话」的问题，也**不该**把它硬塞进 `Derived`（那会让 `--derived` 也依赖 state 的额外
//! 计算，破坏「post 是 state 的函数」这条既有理由）。见 `ship-blueprint-spec.md` §5.3/§9.10。

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
    derived_rows_all(dir, table)
        .into_iter()
        .filter(|r| r["round"] == serde_json::json!(round))
        .collect()
}

/// 同一张表的**全部回合**（不按回合过滤）——「这一局里到底发生过什么」要问它，
/// 而不是问最后一回合那一帧（最后一帧有没有仗打取决于当回合态势，钉它会随轨迹漂移翻车）。
fn derived_rows_all(dir: &std::path::Path, table: &str) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join("idx").join(format!("{table}.jsonl"))).unwrap();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .collect()
}

// ⚠ **两张翻译函数已经删掉了**（2026-10，`feature/pre-post-unify`）：`res_map`（null ↔ `{}`）
// 与 `gov_of`（治理的"存在性"翻译）存在的唯一理由，是「`flow` 表缺键」与「`metrics` 缺省值」
// 曾经是两套约定——`flow.jsonl` 说覆盖 0%、同回合的 `metrics.factions[].governance_coverage`
// 说 100%，两个读面各说各话。现在两个读面**读的是同一份 `RoundView`**：每势力一行、默认值由
// `observe` 一处给出，所以这里可以直接**逐值相等**地比，不需要任何翻译。

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

    // 3) 主流最后一行的 `view`：必须等于 ckpt 里整个 `post`（逐值相等，不是"差不多"）。
    let main = last_main_row(&out);
    assert_eq!(main["round"], serde_json::json!(6));
    assert_eq!(
        main["view"], v["post"],
        "main.jsonl 的 view 与 ckpt 的 post 不是同一个值——两个读面各说各话"
    );

    // 4) **过程量表**最后回合的每一行：必须等于 ckpt 里 `post.factions[<势力>]` 的对应列。
    //    两个读面读的是同一份视图 ⇒ 直接逐值相等，不再需要任何"存在性翻译"。
    let proc = derived_rows(&out, "faction_process", 6);
    assert!(!proc.is_empty(), "idx/faction_process.jsonl 在最后一回合应有行");
    let rows = &v["post"]["factions"];
    let mut checked = 0usize;
    let mut governance_ran = 0usize;
    let mut admin_seen = 0usize;
    for row in &proc {
        let fid = row["faction_id"].as_str().unwrap();
        let expect = &rows[fid];
        assert!(
            !expect.is_null(),
            "ckpt 的 post.factions 里没有 {fid} 的行——视图丢了过程量"
        );
        assert_eq!(row["upkeep"], expect["upkeep"], "{fid} 的 upkeep 两个读面不一致");
        assert_eq!(
            row["production"], expect["production"],
            "{fid} 的 production 两个读面不一致"
        );
        assert_eq!(
            row["governance_total"], expect["governance_cost"],
            "{fid} 的治理总开销两个读面不一致"
        );
        assert_eq!(
            row["governance_coverage"], expect["governance_coverage"],
            "{fid} 的治理覆盖率两个读面不一致"
        );
        // B1：治理的拆分、人口超载倍率、两个全国项必须跨进程逐值相同。
        // ⚠ 首都评估/迁都**不在**这一行里：它是稀疏的判定，住在 `decisions` 数组（下面单独钉）。
        for col in [
            "governance_admin",
            "governance_entertainment",
            "governance_scale",
            "ideology_loyalty_penalty",
            "capital_loyalty_bonus",
        ] {
            assert_eq!(row[col], expect[col], "{fid} 的 {col} 两个读面不一致");
        }
        // B2（钱去哪了）：花掉的投资/建造预算、欠付维护费与生锈比例也必须跨进程逐值相同。
        // ⚠ 「批了多少」**不在**这一行里——限额是控制面的持久叶（join `derived.control` 的
        // `kind='investment_budget'`/`'construction_budget'`），两份相减才是「没花掉的」。
        // 这条守卫管的是「已花」那一半；它是否**真的非零**由 `src/tests/sim/spending.rs` 钉
        // （那里跑 8 回合，确保真有花钱的回合），这里只保证两个读面给同一个数。
        for col in [
            "investment_spent",
            "construction_spent",
            "upkeep_unpaid",
            "fleet_rust",
        ] {
            assert_eq!(row[col], expect[col], "{fid} 的 {col} 两个读面不一致");
        }
        if expect["governance_admin"].as_f64().unwrap_or(0.0) > 0.0 {
            admin_seen += 1;
        }
        if expect["governance_cost"].as_f64().unwrap_or(0.0) > 0.0 {
            governance_ran += 1;
        }
        checked += 1;
    }
    assert!(checked >= 2, "只比对到 {checked} 个势力——守卫太空（至少要有 >=2 个）");
    assert!(
        governance_ran >= 1,
        "这一回合没有任何势力真的跑过治理——这条守卫会退化成空转"
    );
    assert!(admin_seen >= 1, "没有任何势力报出行政开销——B1 那几列等于空转");
    // 首都判定是**稀疏数组**（不在每势力一行里）：形状必须是数组，且缺席表示「既没评估也没迁」。
    let cap = &v["post"]["decisions"]["capital"];
    assert!(cap.is_array(), "view.decisions.capital 必须是数组（稀疏判定）");
    for c in cap.as_array().unwrap() {
        assert!(c["faction"].is_string(), "首都判定行缺 faction：{c}");
        assert!(
            c["reviewed"] == serde_json::json!(true) || !c["relocated_to"].is_null(),
            "首都判定行既没评估也没迁，不该占位：{c}"
        );
    }

    // 5) 城的过程量表同理，且必须真有带产出的行（否则等于没检查）。
    let city_proc = derived_rows(&out, "city_process", 6);
    let with_prod: Vec<&serde_json::Value> = city_proc
        .iter()
        .filter(|r| r["production"].as_object().map(|o| !o.is_empty()).unwrap_or(false))
        .collect();
    assert!(!with_prod.is_empty(), "最后一回合没有任何带产出的城行");
    let mut targets_seen = 0usize;
    let mut hubs_seen = false;
    for row in with_prod {
        let cid = row["city_id"].as_str().unwrap();
        assert_eq!(
            row["production"], v["post"]["cities"][cid]["production"],
            "{cid} 的产出两个读面不一致"
        );
        // B1：忠诚目标值分项——平铺列 vs 视图里的嵌套对象，逐个同源。
        let lt = &v["post"]["cities"][cid]["loyalty_target"];
        assert_eq!(
            row["loyalty_target_effective"], lt["effective"],
            "{cid} 的忠诚目标值两个读面不一致"
        );
        assert_eq!(
            row["loyalty_target_distance"], lt["distance"],
            "{cid} 的忠诚距离项两个读面不一致"
        );
        if lt["effective"].as_f64().unwrap_or(0.0) > 0.0 {
            targets_seen += 1;
        }
        // B2：产出与建造的中间量——用工系数 / 住房容量 / 是否集散地 / 每舰级造舰进度
        // （`build` 是嵌套对象：平铺列里就是它本身，逐值相同）。
        let b2 = &v["post"]["cities"][cid];
        assert_eq!(row["labor"], b2["labor"], "{cid} 的用工系数两个读面不一致");
        assert_eq!(
            row["housing_capacity"], b2["housing_capacity"],
            "{cid} 的住房容量两个读面不一致"
        );
        assert_eq!(row["is_hub"], b2["is_hub"], "{cid} 的集散地标记两个读面不一致");
        assert_eq!(row["build"], b2["build"], "{cid} 的造舰进度两个读面不一致");
        // 中性值约定：用工系数**永远不该是 0**（0 会被读成「全城没人上工」，中性值是 1.0）。
        assert!(
            row["labor"].as_f64().unwrap_or(0.0) > 0.0,
            "{cid}: 用工系数落到了 0——中性值约定被破坏了（应为 1.0 起步）"
        );
        let hubs_seen_here = row["is_hub"] == serde_json::json!(true);
        hubs_seen |= hubs_seen_here;
    }
    assert!(targets_seen >= 1, "没有任何城报出忠诚目标值——B1 那几列等于空转");
    // 集散地至少要有真的一处（否则「产出直进势力池」这条路永远是 false，列等于空转）。
    assert!(hubs_seen, "没有任何城被标成集散地（首都）——`is_hub` 那列等于空转");

    // 6) 控制面表也要在（读面即写面的 tidy 版），并且 join 列真的存在于主流。
    for table in ["control", "scope"] {
        assert!(!derived_rows(&out, table, 6).is_empty(), "idx/{table}.jsonl 在最后一回合应有行");
    }
    assert!(main["faction_ids"].is_array(), "main 行要有 faction_ids（派生表的 join 列）");
}

/// **投影一份 checkpoint 时，起点那一行的流量必须是那一回合的真数**，不是 0。
///
/// 档里的 `post` 就是「产生这个状态的那一回合」的派生态，而 `--start ckpt --round 0 --index`
/// 写出的回合 0 行的 state 正是那个状态。若这里退回 `view_from_state`，agent 会看到
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
        main["view"], v["post"],
        "起点回合的视图必须是档里那一回合的，不是重算（重算会把过程量抹成 0）"
    );
    let proc = derived_rows(&proj, "faction_process", round);
    assert!(!proc.is_empty(), "起点回合应有过程量行");
    let mut nonzero = 0usize;
    for row in &proc {
        let fid = row["faction_id"].as_str().unwrap();
        assert_eq!(row["upkeep"], v["post"]["factions"][fid]["upkeep"], "{fid} 的 upkeep 应为档里的真数");
        assert_eq!(
            row["production"], v["post"]["factions"][fid]["production"],
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
        derived_rows(&fresh, "faction_process", 0)
            .iter()
            .all(|r| r["upkeep"].as_f64().unwrap_or(0.0) == 0.0),
        "全新开局的回合 0 不该有过程量"
    );
}

#[test]
fn derived_without_checkpoint_says_it_was_recomputed() {
    let st = run(&["--seed", "7", "--derived"]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    assert_eq!(v["source"], serde_json::json!("state"));
    assert!(
        v["note"].is_string(),
        "按当前状态重算时必须给出 note，别让全 0 的过程量看起来像事实"
    );
    // 没有档就没有本回合过程量：每个势力的过程列都必须是 0/空（而不是报错/编数）。
    let rows = v["post"]["factions"].as_object().unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.values().all(|r| r["upkeep"].as_f64().unwrap_or(0.0) == 0.0
            && r["production"].as_object().map(|o| o.is_empty()).unwrap_or(true)),
        "按状态重算时不该凭空出现过程量"
    );
    // 但观测部分仍然是真的（从当前状态汇总），不该是空壳。
    assert!(rows.len() >= 2);
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

    // 跑到**确实有仗打**的回合——否则战斗分支永远走不到。跑 60 回合：「seed 7 在 r3 开战」
    // 是旧轨迹上的事实，几次有意为之的行为改动（造舰动机、风格/设计图的执行者）都把它推后了。
    // 守卫要防的是「表退化成空的」，不是「某个特定回合有仗打」⇒ 给足回合数。
    let st = run(&["--seed", "7", "--round", "60", "--index", out_s, "--save", ckpt_s]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));

    let st = run(&["--start", ckpt_s, "--derived"]);
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    let round = v["round"].as_u64().unwrap();
    let dec = &v["post"]["decisions"];
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
    //
    // ⚠ **范围是整局而不是最后一回合**：最后一回合放没放炮取决于当回合的态势，把它钉死会让
    // 这条守卫随着世界轨迹漂移而随机翻车——实测踩过（造舰动机那次改动之后，seed 7 的 r20
    // 恰好一炮没放，而前后各回合照打）。守卫要防的是「表退化成空的」，不是「r20 有仗打」。
    let all_rounds = derived_rows_all(&out, "decisions");
    let verdicts: std::collections::BTreeSet<&str> = all_rounds
        .iter()
        .filter(|r| r["kind"] == serde_json::json!("ship_order"))
        .map(|r| r["verdict"].as_str().unwrap())
        .collect();
    // 「打了仗」= 打船（`engage`）**或**打城（`bombard`）：只钉 `engage` 会让守卫依赖
    // 「接战恰好发生在这一局」这个轨迹细节（实测：合并后这 60 回合里只有攻城、没有接战）。
    // 它要证明的是**判定表里真有战斗**，而不是某一条分支必须出现。
    assert!(
        verdicts.contains("engage") || verdicts.contains("bombard"),
        "seed 7 的前 60 回合里应当真的打过仗（接战或攻城），实际只见到 {verdicts:?}"
    );
    assert!(
        order_rows.len() >= 4,
        "最后一回合的逐舰判定只有 {} 条——守卫太空",
        order_rows.len()
    );

    // B2 的防空转：**整局**（60 回合）里必须真的花过钱、也真的有过造舰进度行——
    // 否则上面那几列只是「空表比空表」，跨进程相等毫无意义。范围取整局的理由同上：
    // 某一回合有没有在建的东西取决于当回合的态势，钉死单帧会随轨迹漂移而翻车。
    let faction_all = derived_rows_all(&out, "faction_process");
    let spend_seen = faction_all.iter().any(|r| {
        ["investment_spent", "construction_spent"].iter().any(|k| {
            r[*k].as_object().map(|o| o.values().any(|v| v.as_f64().unwrap_or(0.0) > 0.0)).unwrap_or(false)
        })
    });
    assert!(spend_seen, "seed 7 的前 60 回合里应当真的花过钱（投资或造舰）——B2 那几列等于空转");
    let city_all = derived_rows_all(&out, "city_process");
    let lines_seen = city_all
        .iter()
        .any(|r| r["build"].as_object().map(|o| !o.is_empty()).unwrap_or(false));
    assert!(lines_seen, "没有任何城报出造舰进度行——`build` 那列等于空转");
    let rust_seen = faction_all.iter().any(|r| r["fleet_rust"].as_f64().unwrap_or(0.0) > 0.0);
    let unpaid_seen = faction_all.iter().any(|r| r["upkeep_unpaid"].as_f64().unwrap_or(0.0) > 0.0);
    assert!(
        rust_seen && unpaid_seen,
        "这 60 回合里应当至少有一家付不起维护费（欠费与生锈两列一起才说明它真的在发生）"
    );
}
