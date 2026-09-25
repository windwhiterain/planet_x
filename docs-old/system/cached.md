# 图里只有一个函数：`cached`

> 2026-09-21（`feature/graph-cached`）。用户口径逐句是：**"没有包装，没有 read_cache，
> 只有一个函数 `cached<F>(f: F)`，就是读一个缓存值，缓存是脏的就调用算子算"**、
> **"脚本给参数就是 rust 给参数"**、**"算子库里接受一个包装对象作为参数就表明这个参数
> 可以是被 cache 的，且包装对象是 nested、递归的，裸值可以直接在脚本里构造包装对象"**。

## 1. 改之前是什么形状

```rust
cook<O: PxOp>(cache, node, inputs: O::Inputs) -> Result<Cooked<O::Payload>, String>
```

参数**不在调用点上**：`cook` 自己按节点名去读 `art/<图>/<节点>.toml`（`cache.params_text`）
→ `toml::from_str::<O::Params>` → 缺文件走 `Default`。于是：

* 脚本给不出参数 —— "这个字段用算出来的值"这件事**没有表达方式**；
* 缓存没有开关 —— 要么走 `cook`（查盘 + 落盘），要么**完全在图外**（自己 `encode`）；
* `Cooked<T>` 只能由 `cook` 造出来 ⇒ 一个"图外算出来的值"**接不到下游**。

## 2. 改之后

```rust
cached<O: PxOp>(cache, node, f: O, params: O::Params, inputs: O::Inputs)
    -> Result<Cooked<O::Payload>, String>
```

* **参数是普通 Rust 值**：`node_params(&graph, "节点")?` 读一份打底（缺文件 → `Default`），
  想改哪个字段就在脚本里改哪个：`Params { zonal: look.stretch, ..node_params(..)? }`。
* **`cached` 不碰 `art/`**：读文件是脚本自己的一步（`node_params` 是"读一个文件 + 解一份 TOML"，
  与缓存无关）。
* **不缓存 = 直接调那个普通函数**：`field::Fbm.render(&params, &(), graph.grid())?` ——
  没有第二个缓存入口、没有 `.uncached()`。
* **裸值能进图**：`Cooked::of(裸值)?` 把它包成"可以进图的东西"，键由**内容**算
  （`Build::encode` → `to_bytes("", &[])` 的 blake3）。
* **包装对象是类型标记**：`HashField for Cooked<T>` 写的是**它的键** ⇒ 参数是 `Cooked<T>`
  就表示"这个参数可以被缓存"，而且嵌进任何参数结构、嵌几层都成立（`Cooked<Cooked<T>>`）。
  `PxOp` / `PxInputs` / 算子实现 / dylib ABI **一个字节都没动**。

## 3. 判据（可重跑）

* **内容逐字节**：六张图 45 个节点的产物 sha256 与改动前逐格相同
  （`target/baseline.tsv` ↔ `target/final.tsv`）—— 键**全换**（契约层多了两个 impl），
  内容一位没动。
* **第二趟全命中**：`px run planet` 打「命中 6、重算 0」。
* `px_graphs/tests/bare_value.rs`（新）：同一份内容 ⇒ 同一个键、裸值真能当上游、
  **内容变了下游的键就变**。8×4 画布。
* `cargo test -p px_graphs -p px_graph`：全过。

## 4. 顺手删掉的一条重复路

`px_volume_schema::params::parse(toml_text)` 是"TOML → 类型化参数"的第二份实现，
只有一个调用者（`clouds.rs` 的判据仪器）—— 改成走 `node_params`，那份实现删了。

## 5. 还欠着的（没做，记在这里）

* `docs/guides/writing-an-operator-library.md` 与 `docs/**/*.md` 里旧写法（`cook::<…>`）
  仍按**历史**读；指南那一篇已按新形状改过，笔记不改（它们是决策记录）。
* `art/anchor/hashes.txt` §三 那六格**没有重登记**（2026-09-20 起的口径：判据 = 内容逐字节，
  而 §3 已经量过内容没变）。
