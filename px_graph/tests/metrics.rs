//! See docs/graph.md
//!
//! The measurement ledger: `finish()` appends one run (a header plus one line per node) to
//! `metrics.jsonl` under the graph's cache directory. These gates cover what a reader would depend
//! on — the sequence counts up across runs even when every run starts in the same second, each line
//! round-trips, and a damaged line is skipped with the sequence still recovered from what remains.
//!
//! `seq` is the run key rather than the wall clock: two runs inside one second are ordinary, and a
//! timestamp that collides cannot order the file.

use std::path::{Path, PathBuf};

use px_graph::driver::{Graph, append_metrics, metrics_path};
use px_graph_schema::{GraphSpec, ManifestEntry};

/// A throwaway directory under the workspace's `target/`, which is tracked by nothing (no roster
/// contains `target/`, so a test's own files here rotate no key).
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graph lives under the workspace")
        .join("target")
        .join("metrics-test")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建得了临时目录");
    dir
}

fn entry(node: &str, key: &str, hit: bool, millis: u64, bytes: u64) -> ManifestEntry {
    ManifestEntry {
        node: node.to_string(),
        op: "field.constant".to_string(),
        op_version: 1,
        key: key.to_string(),
        hit,
        millis,
        bytes,
        detail: String::new(),
    }
}

fn graph_in(cache_root: &Path) -> Graph {
    Graph::with_cache_root(
        GraphSpec {
            name: "metrics".to_string(),
        },
        cache_root.to_path_buf(),
    )
}

fn lines(cache_root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(metrics_path(cache_root, "metrics")).expect("账本读得到");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn value(line: &str) -> serde_json::Value {
    serde_json::from_str(line).expect("每一行都是 JSON")
}

/// Just past the line limit, so the rotation gate crosses the threshold without writing a real
/// 4096-run ledger. The limit itself is the driver's `METRICS_MAX_LINES`.
const METRICS_MAX_LINES_TEST: u64 = 4097;

#[test]
fn a_run_is_recorded_as_a_header_and_one_line_per_node() {
    let dir = scratch("round-trip");
    let graph = graph_in(&dir);
    graph.record(entry("a", "aa", false, 12, 100));
    graph.record(entry("b", "bb", true, 0, 200));
    graph.finish();

    let lines = lines(&dir);
    assert_eq!(lines.len(), 3, "一条 run 头 + 两个节点：{lines:?}");

    let header = value(&lines[0]);
    assert_eq!(header["seq"], 1);
    assert_eq!(header["graph"], "metrics");
    assert_eq!(header["node_count"], 2);
    let started = header["started"].as_str().expect("started 是字符串");
    assert!(
        started.ends_with('Z') && started.len() == 20 && started.contains('T'),
        "started 要是 ISO-8601 UTC：{started}"
    );

    let first = value(&lines[1]);
    assert_eq!(first["seq"], 1);
    assert_eq!(first["node"], "a");
    assert_eq!(first["key"], "aa");
    assert_eq!(first["hit"], false);
    assert_eq!(first["cook_millis"], 12);
    assert_eq!(first["bytes"], 100);

    let second = value(&lines[2]);
    assert_eq!(second["node"], "b");
    assert_eq!(second["hit"], true, "命中也要记：§8 那类争议靠它才查得动");
    assert_eq!(second["cook_millis"], 0);
}

#[test]
fn a_second_run_appends_and_the_sequence_counts_up() {
    let dir = scratch("append");
    for expected in 1..=3_u64 {
        let graph = graph_in(&dir);
        graph.record(entry("only", "11", false, 1, 1));
        graph.finish();
        let lines = lines(&dir);
        assert_eq!(
            lines.len() as u64,
            expected * 2,
            "第 {expected} 轮之后应有 {expected} 组（头+节点）"
        );
        assert_eq!(
            value(lines.last().expect("有行"))["seq"],
            expected,
            "第 {expected} 轮"
        );
    }
}

#[test]
fn a_damaged_line_is_skipped_and_the_sequence_still_advances() {
    let dir = scratch("damaged");
    let path = metrics_path(&dir, "metrics");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "{\"seq\": 7, \"graph\": \"metrics\", \"started\": \"2026-01-01T00:00:00Z\", \"node_count\": 1}\n\
         {\"seq\": 7, \"node\": \"ok\", \"key\": \"11\", \"hit\": false, \"cook_millis\": 1, \"bytes\": 1}\n\
         { this line is not JSON at all\n",
    )
    .expect("坏账本写得进");

    let graph = graph_in(&dir);
    graph.record(entry("next", "22", false, 2, 2));
    graph.finish();

    let lines = lines(&dir);
    let last = value(lines.last().expect("有行"));
    assert_eq!(
        last["seq"], 8,
        "坏行之后的序号要接着 7 走，不能因为读不动就重置：{lines:?}"
    );
    assert_eq!(last["node"], "next");
}

#[test]
fn an_empty_or_absent_ledger_starts_at_one() {
    let dir = scratch("absent");
    let graph = graph_in(&dir);
    graph.record(entry("first", "33", false, 3, 3));
    graph.finish();
    assert_eq!(value(&lines(&dir)[0])["seq"], 1);
}

#[test]
fn writing_reports_its_own_failure() {
    let dir = scratch("unwritable");
    // A file where the directory should be: creating `metrics.jsonl` under it cannot succeed.
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let err = append_metrics(&blocked, "metrics", &[entry("a", "aa", false, 1, 1)])
        .expect_err("写不进去时必须报错，不许静默");
    assert!(err.contains("blocked"), "错误要指出是哪个路径：{err}");
}

#[test]
fn a_small_ledger_is_left_alone_and_a_full_one_is_rotated() {
    let dir = scratch("rotate");
    let path = metrics_path(&dir, "metrics");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    // Under both limits: appended to, nothing renamed.
    let small = "{\"seq\": 1, \"graph\": \"metrics\", \"started\": \"2026-01-01T00:00:00Z\", \"node_count\": 1}\n";
    std::fs::write(&path, small).unwrap();
    append_metrics(&dir, "metrics", &[entry("a", "11", false, 1, 1)]).expect("追加得进");
    assert!(!path.with_extension("jsonl.1").exists(), "小账本不该轮转");
    assert_eq!(lines(&dir).len(), 3, "原来那行 + 新的一轮（头+节点）");

    // Past the line limit (kept to just over, so the gate stays fast): rotated, then appended fresh.
    let mut big = String::new();
    for index in 0..METRICS_MAX_LINES_TEST {
        big.push_str(&format!(
            "{{\"seq\": {}, \"node\": \"n{index}\", \"key\": \"{}\", \"hit\": false, \"cook_millis\": 1, \"bytes\": 1}}\n",
            index / 2 + 1,
            "0".repeat(64)
        ));
    }
    std::fs::write(&path, &big).unwrap();
    append_metrics(&dir, "metrics", &[entry("after", "44", false, 4, 4)]).expect("追加得进");

    let rotated = path.with_extension("jsonl.1");
    assert!(rotated.is_file(), "上一份该在 {} 里", rotated.display());
    assert_eq!(
        std::fs::read_to_string(&rotated).unwrap().lines().count(),
        METRICS_MAX_LINES_TEST as usize,
        "轮转过去的正是上一份"
    );
    let fresh = value(&lines(&dir)[0]);
    assert!(
        fresh["seq"].as_u64().unwrap() > 1,
        "轮转不该把序号打回 1：{fresh:?}"
    );
}
