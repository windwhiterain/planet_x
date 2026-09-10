//! **指令读面「每舰一行」+ 不动点**——用**真实 CLI** 端到端钉住（不是只跑单测）。
//!
//! `--control` 是「读面即写面」的兑现处：dump → 原样回传 → 再 dump，两次必须**逐字节相同**。
//! 这条承诺只在**跨进程、跨 checkpoint** 的路径上才真正被考验（`--apply` 会写状态、
//! `--save` 会落盘），所以它值得一个集成测试。
//!
//! 本轮新增的两件事（`control-live-layers.md` §13）：
//! 1. `ship_orders` 的读面**每舰一行**（以前只列**有叶**的舰 ⇒ 叶被 `remove` 掉之后那艘舰
//!    整行从控制树里消失，玩家/agent 再也没法在界面上单独给它设归属）；
//! 2. `behavior` 是**有效值**，链上没人说话时是 `null`——于是回传那一行**不许建叶**
//!    （否则 `null` 会被静默变成 `Some(Idle)`，读面就不再是不动点）。

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_planet_x"))
}

/// 测试专属临时目录（同进程内各测试 pid 相同 ⇒ 名字里带 tag）。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let d =
            std::env::temp_dir().join(format!("planet_x_readface_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
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

/// `--control` 的输出（stdout 一行 JSON）。
fn control(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "--control 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout 必须是 UTF-8")
}

/// 一个 checkpoint 的状态读数（`--round 0` 不推进时间，只打印当前状态）。
fn state_of(ckpt: &str) -> serde_json::Value {
    let out = control(&["--start", ckpt, "--round", "0"]);
    let line = out
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("状态流至少一行");
    serde_json::from_str(line).expect("状态流是 JSON")
}

fn faction_ships(state: &serde_json::Value, fid: &str) -> Vec<String> {
    state["ships"]
        .as_array()
        .expect("ships 是数组")
        .iter()
        .filter(|s| s["faction_id"] == serde_json::json!(fid))
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

fn orders_row<'a>(surface: &'a serde_json::Value, fid: &str) -> &'a Vec<serde_json::Value> {
    surface["control"]
        .as_array()
        .expect("control 是数组")
        .iter()
        .find(|f| f["faction_id"] == serde_json::json!(fid))
        .unwrap_or_else(|| panic!("控制面里必须有势力 {fid}"))["ship_orders"]
        .as_array()
        .expect("ship_orders 是数组")
}

#[test]
fn every_ship_gets_an_order_row_and_the_template_is_a_fixed_point() {
    let d = Scratch::new("fixed_point");
    let ckpt0 = d.path("ckpt0.ron");
    let ckpt1 = d.path("ckpt1.ron");
    let ckpt2 = d.path("ckpt2.ron");
    let t0 = d.path("t0.json");
    let t1 = d.path("t1.json");

    // 1) 一个小而真实的世界。
    let out = run(&[
        "--seed",
        "42",
        "--round",
        "12",
        "--save",
        ckpt0.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "跑局失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ckpt0s = ckpt0.to_str().unwrap();

    // 挑一艘有叶的舰，把它那片叶**删掉**——这就是「从没被点名过」的状态
    // （也是 kit 的 `remove_ship_order` / web 的「恢复出厂值」之后的状态）。
    let state = state_of(ckpt0s);
    let fid = state["ships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["faction_id"].as_str().unwrap().to_string())
        .find(|f| faction_ships(&state, f).len() >= 2)
        .expect("至少有一个势力有 2 艘以上舰");
    let ours = faction_ships(&state, &fid);
    let vanished = ours[0].clone();

    let rm = d.path("rm.json");
    std::fs::write(
        &rm,
        serde_json::json!({"control": [{"faction_id": fid, "ship_orders": [{"ship": vanished, "remove": true}]}]})
            .to_string(),
    )
    .unwrap();
    let out = run(&[
        "--start",
        ckpt0s,
        "--apply",
        rm.to_str().unwrap(),
        "--round",
        "0",
        "--save",
        ckpt1.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "删叶失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("NOTE_APPLY_REMOVED"),
        "删叶必须留下回执（删的是叶，不是值）"
    );
    let ckpt1s = ckpt1.to_str().unwrap();

    // 2) 读面：**每舰一行**，且那艘被删过叶的舰仍然在。
    let before = control(&["--start", ckpt1s, "--control"]);
    let surface: serde_json::Value = serde_json::from_str(&before).unwrap();
    let rows: Vec<String> = orders_row(&surface, &fid)
        .iter()
        .map(|r| r["ship"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        rows, ours,
        "指令读面必须**每舰一行**且顺序与 `state.ships` 一致（含叶被删掉的「{vanished}」）"
    );
    let row = |s: &serde_json::Value, ship: &str| -> serde_json::Value {
        orders_row(s, &fid)
            .iter()
            .find(|r| r["ship"] == serde_json::json!(ship))
            .unwrap_or_else(|| panic!("读面里必须有「{ship}」这一行"))
            .clone()
    };
    assert_eq!(
        row(&surface, &vanished),
        serde_json::json!({"ship": vanished, "behavior": serde_json::Value::Null, "mode": "Inherit"}),
        "叶被删掉的舰：`behavior` 是 `null`（链上没人说话）、`mode` 是 `Inherit`"
    );
    for other in ours.iter().filter(|s| **s != vanished) {
        assert!(
            !row(&surface, other)["behavior"].is_null(),
            "「{other}」的叶还在，有效值必须是一个真行为（不是 null）"
        );
    }

    // 3) **原样回传**这一面模板，再读一次：必须逐字节相同。
    std::fs::write(&t0, &before).unwrap();
    let out = run(&[
        "--start",
        ckpt1s,
        "--apply",
        t0.to_str().unwrap(),
        "--round",
        "0",
        "--save",
        ckpt2.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "回传模板失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("WARN_APPLY_SKIPPED"),
        "模板回传不许丢叶子: {stderr}"
    );
    let after = control(&["--start", ckpt2.to_str().unwrap(), "--control"]);
    std::fs::write(&t1, &after).unwrap();
    assert_eq!(before, after, "读面不是不动点：原样回传改变了它自己的形状");

    // 4) 「`behavior: null` ⇒ 不建叶」这条规则是上一条断言的地基：显式再钉一次
    //    （读面相同 + 那片叶仍然不存在 = 回传没有偷偷把 null 变成 Idle）。
    let surface2: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(
        row(&surface2, &vanished),
        serde_json::json!({"ship": vanished, "behavior": serde_json::Value::Null, "mode": "Inherit"}),
        "回传之后那一行还必须说「链上没人说话」"
    );
    // `--control` 是纯读，状态流才是真相：拿**回传之后**的状态流再确认一遍每舰一行。
    let state2 = state_of(ckpt2.to_str().unwrap());
    assert_eq!(faction_ships(&state2, &fid), ours, "回传模板不许增删舰");
    assert_eq!(
        orders_row(&surface2, &fid).len(),
        ours.len(),
        "回传之后仍然每舰一行"
    );
}
