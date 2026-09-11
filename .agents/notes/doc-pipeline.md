# 文档注释 → description 管线（`///` → 悬停弹窗文案）

> 状态：`[x]` 已实现（分支 `feature/doc-pipeline`，**未提交**）。2026-10 用户裁决：**升级 schemars**。
> 涉及：`Cargo.toml` / `Cargo.lock`（0.8.22 → 1.2.2）、新模块 `src/schema.rs`、
> `src/agent.rs` / `src/control/view.rs` / `src/tests/model/neutral.rs` 的四处 `schema_for!` 调用点、
> `play/tests/g4_spec.py` §5b（新守卫）、`play/tests/_g4_negative.py` ㉓㉔（守卫的量具）。

## 1. 现象与根因

`planet_x --nouns`（悬停弹窗的唯一文字来源）里有一批 `description` **以单个 `*` 开头**：
源码明明是 `/// **完整的世界快照**（…`，弹窗拿到的是 `*完整的世界快照**（…` ⇒ **坏 markdown**。
实测升级前：以 `**` 开头 94 条、以单个 `*` 开头 **46** 条（46/46 全是**单行**注释）。

根因在 schemars 0.8.22 的 derive（`schemars_derive-0.8.22/src/attr/doc.rs::get_doc`）里的一段
向后兼容 hack：

```rust
// Added for backward-compatibility, but perhaps we shouldn't do this
if lines.iter().all(|l| l.starts_with('*')) {
    for line in lines.iter_mut() { *line = line[1..].trim() }
    ...
}
```

它把「文档**所有行**都以 `*` 开头」当成 `/** … */` 风格并逐行剥掉一个 `*`。于是**单行**注释
若以 `**加粗**` 开头就被误判；多行注释的后续行通常不以 `*` 开头 ⇒ 幸免。

## 2. 选的路：升级 schemars（0.8.22 → 1.2.2）

**1.x 里那段逻辑没了**，而且是整块重写。`schemars-1.2.2/src/_private/rustdoc.rs`：

```rust
pub const fn get_title_and_description(doc: &str) -> (&str, &str) {
    let doc_bytes = trim_ascii(doc.as_bytes());
    if !doc_bytes.is_empty() && doc_bytes[0] == b'#' { /* 首行当 title */ }
    else { ("", to_utf8(doc_bytes)) }      // ← 只 trim，没有剥 `*`
}
```

取证（可当场复现）：

```bash
grep -rn "starts_with('\*')\|line\[1\.\.\]" ~/.cargo/registry/src/*/schemars*/src/**/*.rs   # 1.x 无命中
python play/tests/_g4_negative.py        # ㉓ 复刻旧 hack 的效果 ⇒ 新守卫自己红
```

### 升级的代价与处理

| 差一点踩的坑 | 处理 |
| --- | --- |
| 1.x 默认方言换成 **2020-12**（`$defs` + `#/$defs/X`），而 `g4_spec.py` / `neutral.rs` / `tip.js` 认的是 0.8 的 draft-07 外形（`definitions`） | `src/schema.rs::of::<T>()` 用 `SchemaSettings::draft07()` **把方言钉死**；全仓 `schema_for!` 只走这一个入口 |
| 1.x 不再把段内换行折成空格 ⇒ 弹窗里每条源码硬换行都变成 `<br>`（420px 宽的弹窗会被折成锯齿） | `src/schema.rs::merge_doc_lines` 补回 0.8 的 `merge_description_lines` 语义（先逐行 `trim`、再按空行分段、段内换行归一为空格、段间保留 `\n\n`） |
| `#[schemars(with = "…")]` / `#[serde(rename)]` / `schema_for!` 的 API 迁移 | **零迁移**：1.x 仍认 `#[serde(rename)]`（`serde_derive_internals` 0.30），`with` 也还在，三处 `#[schemars(with)]` 原样可用 |
| 别的东西钉死 0.8 | 无（`grep` 过全仓只有根 crate 用 schemars） |

**升级后的对账结果**（最硬的证据）：把升级前后的 `--nouns` 逐条比，
**664 条共有 description 里只有 46 条不同，且 46 条全是那批被剥掉 `*` 的**（按空白归一后比 = 0 条差异）。
其余结构性差异见 §4。

## 3. 守具：`g4_spec.py` §5b「文档对账」

单看 `--nouns` **看不出对错**（它自洽），所以守卫拿**源码**当尺子：扫描 `src/**/*.rs` 的
`struct`/`enum` + 紧挨着的 `///`（`#[serde(rename)]` 换键名），然后

* **正向**：三份 schema 的**每一条** `description`（根 `title` 指的类型 / 定义级 / 字段级 /
  `oneOf` 变体级）都要在源码里找到出处，且**空白归一后逐字相等**；
* **反向**：schema 里出现的每个结构体，**源码带 `///` 的字段都真的发射了**（丢文档 = 弹窗少一段话）；
* **防空转**：实测 **615 条**对账（下限 550）、**395 个**带注释字段（下限 150）、**17 个**源文件（下限 10）；
  解析不出来的发射项**一律判红**（`Control2…6` 这类 schemars 给重名泛型实例编的号由 `resolve()` 归回 `Control`）。

量具：`_g4_negative.py` ㉓（把一条 `**加粗**` 开头的 description 剥掉一个 `*`）与 ㉔（悄悄删掉一条
description）都要求「文档对账」那一族**自己**红——实测两条都咬住（全组 30 个注入错全咬住）。

⚠ 口径边界：`projection` 的 `column_docs` / 列内联 `description` 是**手写**文案、不走 schemars，
不在本条范围（由 §5 名词覆盖率盯着）；枚举**变体级**的 `///` 只查正向，理由见 §4。

## 4. 剩下的形状差异（都是语义等价或"更正确"，但是**变了**）

与 0.8 的 `--nouns` 逐键比，除那 46 条文案外还有：

1. `required` 数组**顺序**不同（JSON Schema 里是集合，无影响）；
2. `minimum: 0.0` → `0`（同一个数，只是 JSON 文本形态）；
3. 单值 `enum: ["X"]` → `const: "X"`（draft-07 下语义相同）；
4. 泛型实例的定义名 `Control_for_double` → `Control2`…（`$ref` 跟着变；消费方按 `$ref` 最后一段取，
   且本仓 `Control<T>` 有 6 个实例）；
5. `anyOf`/`allOf` 少一层无意义的嵌套（0.8 `anyOf[0].anyOf=[{$ref},{null}]` → 1.x `anyOf[0].$ref`）；
6. ⚠ **信息净减**：`DeathCause` / `SpawnVia` / `FoundingHow` 三个 enum 的 `oneOf`（含变体名与变体
   `///`）**没了**——它们带 `#[serde(into = "String", try_from = "String")]`，1.x 的 derive 认这对
   属性 ⇒ 直接发 `{"type":"string"}`（`schemars_derive-1.2.2/src/schema_exprs.rs` 的
   `else if let (Some(from), Some(into)) = …`）。影响的 14 条 `description` 是**变体级**的，
   弹窗按**字段名**取词条 ⇒ 本来就不显示；而 0.8 那份 `oneOf` 里的枚举值写的是 Rust 变体名
   （`"Combat"`），**与线上的 snake_case 串（`"combat"`）不符——旧的是错的**。折中：不手写镜像
   schema（那是本仓最忌讳的漂移面），把这条差异记在这里。

## 5. 还没做的

* `DeathCause`/`SpawnVia`/`FoundingHow` 的**变体枚举值**能不能重新发出去（现在 `--schema` 只说
  `type: string`，agent 无法从 schema 里枚举合法值）；要做得让 `stringly_unit_enum!` 顺手生成
  `JsonSchema`（它已经知道 wire 串，值不会漂），但变体 `///` 拿不到（宏看不见注释）。
* 若哪天想回到"忠实保留源码换行"的弹窗排版：删掉 `src/schema.rs::merge_doc_lines` 的调用即可
  （一处，`g4_spec.py` 的判据按空白归一，不会因为这一改红）。
