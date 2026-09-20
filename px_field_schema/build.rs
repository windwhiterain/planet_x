//! **场域声明的源码指纹** —— `px_op!` 里那句 `DECL_HASH = env!("PX_SOURCE_HASH")` 要它。
//!
//! ⚠ 它为什么必须存在：泛型实例（`px_inst!`）的 key 必须覆盖"**声明所在的这一份源码**"
//!   —— 接口哈希只哈希三个**类型名**，不含字段布局；往声明里的输入 struct 加一个字段时，
//!   图程序会重编而实例库的键不变 ⇒ 复用一份按旧布局编出来的 DLL 就是越界读写
//!   （`.agents/notes/art/19-generic-inst.md` §177 / §179.3）。
//!
//! 实现在 `px_fingerprint`（**rlib**：build.rs 与运行期代码共用同一份算法）。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate(&[]);
}
