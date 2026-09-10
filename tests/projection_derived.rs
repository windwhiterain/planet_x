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
        assert_eq!(&row["governance_total"], &post_flow["governance"][fid]["total"]);
        assert_eq!(&row["governance_coverage"], &post_flow["governance"][fid]["coverage"]);
        checked += 1;
    }
    assert!(checked >= 2, "只比对到 {checked} 个势力——守卫太空（至少要有 >=2 个）");

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
