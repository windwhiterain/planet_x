//! **声明 ↔ 实现**那条线的门。
//!
//! 病根的形状换过三轮，现在是：声明住 schema（`px_*_schema/src/ops.rs` 里那几行 `px_op!`），
//! 实现住 `px_*_op` 的 dylib（`px_body!` 导出一个符号，符号名 = `库名__算子名`）。
//! 编译器**看不住**这条线 —— 它是两个字符串拼出来的：
//!
//! * 声明说"去 `px_field_op` 取 `Fbm` 那个符号"
//! * 实现说"我叫 `px_field_op`，我导出 `Fbm`"
//!
//! 只有装载的那一刻才知道对不对得上。所以这道门**真的去装载**：每一个声明过的算子，
//! 都要能从它那份实现库里取到函数指针（连带把契约握手也走一遍）。
//!
//! ⚠ 前提：实现库得先在盘上（`cargo build` / `cargo test`（默认 members）会编它们；
//! 只 `-p px_graphs` 时不会 —— 那时这道门会给你一句带命令的报错）。

use px_graph_schema::PxOp;
use px_graph_schema::ops;

/// 每一个声明过的算子：从它的库里取符号。取不到就是"声明与实现分了家"。
#[test]
fn every_declared_operator_loads_from_its_library() {
    // ⚠ 一个都不能漏：漏掉的那个正是"改了名字没人发现"的候选。
    px_graph_schema::ops::body::<px_field_schema::ops::Constant>().expect("field.constant");
    px_graph_schema::ops::body::<px_field_schema::ops::Fbm>().expect("field.fbm");
    px_graph_schema::ops::body::<px_field_schema::ops::Ridged>().expect("field.ridged");
    px_graph_schema::ops::body::<px_field_schema::ops::Remap>().expect("field.remap");
    px_graph_schema::ops::body::<px_field_schema::ops::Gradient>().expect("field.gradient");
    px_graph_schema::ops::body::<px_field_schema::ops::Mix>().expect("field.mix");
    px_graph_schema::ops::body::<px_field_schema::ops::Warp>().expect("field.warp");
    px_graph_schema::ops::body::<px_volume_schema::ops::CloudCoarse>().expect("cloud.coarse");
    px_graph_schema::ops::body::<px_mesh_schema::ops::CubeSphere>().expect("mesh.cubesphere");
    px_graph_schema::ops::body::<px_mesh_schema::ops::Proxy>().expect("mesh.proxy");
}

/// **实现的身份是运行期读出来的**（图程序不重编也能看见它换了）。
///
/// 这条同时钉住"库里有身份符号"与"指纹形状是十六进制"两件事。
#[test]
fn every_library_reports_its_own_source_hash() {
    for lib in ["px_field_op", "px_volume_op", "px_mesh_op"] {
        let hash = match lib {
            "px_field_op" => <px_field_schema::ops::Fbm as PxOp>::source_hash(),
            "px_volume_op" => <px_volume_schema::ops::CloudCoarse as PxOp>::source_hash(),
            _ => <px_mesh_schema::ops::Proxy as PxOp>::source_hash(),
        }
        .unwrap_or_else(|err| panic!("{lib} 的身份读不到：{err}"));
        assert_eq!(hash.len(), 64, "{lib} 的指纹应当是 64 位十六进制：{hash}");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "{lib} 的指纹不是十六进制：{hash}"
        );
    }
}

/// 三个库的身份**互不相同**：它们各自的源码指纹覆盖的是各自那份源码。
///
/// ⚠ 这一条顺带证明"实现那一半真的进了键"：三个库都链同一份契约，
///   如果指纹只覆盖契约，这三个值会一模一样。
#[test]
fn the_three_libraries_have_distinct_identities() {
    let field = ops::source_hash("px_field_op").expect("field 库身份");
    let volume = ops::source_hash("px_volume_op").expect("volume 库身份");
    let mesh = ops::source_hash("px_mesh_op").expect("mesh 库身份");
    assert_ne!(
        field, volume,
        "场库与体积库的身份相同 —— 指纹没覆盖到各自的源码"
    );
    assert_ne!(field, mesh, "场库与网格库的身份相同");
    assert_ne!(volume, mesh, "体积库与网格库的身份相同");
}
