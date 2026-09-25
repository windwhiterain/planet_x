use px_cook::inst::{BuildGraph, gate_ready};
use px_graphs::insts;

#[test]
fn the_instance_tier_refuses_a_missing_library_with_the_stage_one_command() {
    let mut graph = BuildGraph::new();
    insts::build(&mut graph);
    let plan = graph.missing();
    let outcome = gate_ready(Some(("gate-test", &graph)), &[]);
    if plan.complete() {
        outcome.expect("七条实例都在盘上时，门不得叫");
    } else {
        match outcome {
            Err(message) => assert!(
                message.contains("px build"),
                "缺实例的提示必须指向 stage 1 的正规命令：{message}"
            ),
            Ok(()) => panic!("缺 {} 条实例却放行 ⇒ 门没起：", plan.missing.len()),
        }
    }
}

#[test]
fn the_named_tier_walks_the_real_handshake() {
    let outcome = gate_ready(None, &["px_no_such_library"]);
    let message = outcome.expect_err("一个不存在的实现库必须被点名");
    assert!(
        message.contains("px_no_such_library"),
        "点名缺失的那一份，而不是笼统地喊门：{message}"
    );
}

#[test]
fn five_named_libraries_pass_the_gate_when_they_are_the_exes_own_products() {
    let outcome = gate_ready(None, &["px_field_op", "px_volume_op", "px_mesh_op"]);
    match outcome {
        Ok(()) => {}
        Err(message) => panic!("命名库与图 exe 是同一套cargo build ⇒ 不该拦：{message}"),
    }
}
