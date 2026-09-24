//! `px_cook` 这一份构建的**工具链四原料** —— 生成实例库时要原样转给那份 cargo。
//!
//! ⚠ 为什么 `px_cook` 也要一个 `build.rs`：**生成本身住在这一层**（`inst::BuildGraph` 的
//!   `compile_missing`，`docs/system/build-graph.md` §187 的 B2 后半）。实例库要导出 `__toolchain_hash`，
//!   而那个哈希是「`rustc -vV` + target + `RUSTFLAGS` + profile」算的 ⇒ 生成它的那一侧必须
//!   知道**当前这份构建**的这四样，才能把同一份转给嵌套的那次 `cargo build`。
//!   ⚠ `cargo:rustc-env` **不跨 crate 传播**（`px_fingerprint` 的文件头记过这条实测），
//!   所以这一份只能自己发；`px_graphs` 那份同名的照旧。
//!
//! ⚠ 与指纹**无关**：这个 `build.rs` 不进任何算子的源码名册（算子不依赖 `px_cook`，
//!   见 `px_graphs/tests/crate_graph.rs`），所以它**不换任何节点键**。
//!
//! ⚠ `CARGO` 也在这儿取：**运行期读 `std::env::var("CARGO")` 是错的** —— 那个变量只保证
//!   在 cargo 起的进程里有，而 stage 1 可能在别处调（`px run --build` 从 cargo run 出来，
//!   但 `px.exe` 直接双击就没有）。编译期捕获这一份与货物自己用的那一份是同一个。

fn main() {
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target = std::env::var("TARGET").unwrap_or_default();
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    println!("cargo:rustc-env=PX_PROFILE={profile}");
    println!("cargo:rustc-env=PX_TARGET={target}");
    println!("cargo:rustc-env=PX_RUSTFLAGS={rustflags}");
    println!("cargo:rustc-env=PX_CARGO={cargo}");
    // target triple 与 host 一样时不用给 cargo 传 `--target`（传了会多一层目录）——
    // 判据是 `rustc -vV` 里那行 `host:`，与 `px_fingerprint::toolchain_hash` 取的同一份。
    println!(
        "cargo:rustc-env=PX_RUSTC_VERSION={}",
        rustc_version(&std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string()))
    );

    // ⚠ 这四样一变，实例库的工具链身份就变 ⇒ 必须重编（与 `px_fingerprint` 同一套口径）。
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO");
    // ⚠ `rustc` 本身换个版本也要重算（`-vV` 是工具链指纹的大头）。
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=PX_RUSTC");
}

/// `rustc -vV` 的全文（取不到就空串：那会让"host 与 target 一样"退化成"不传 `--target`"，
/// 而那正是 host 构建的正常形状）。
fn rustc_version(rustc: &str) -> String {
    std::process::Command::new(rustc)
        .arg("-vV")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}
