# 当前状态（v2，2026-09-20）—— 进门先看这一页

> 这份是**入口**：下面每一条都是**今天盘上成立**的口径。历史与设计推演在 `18`–`21`（那几份保留原文，
> 读的时候先看它们的过期标记）。旧入口 `10-handoff.md` 描述的是本轮之前的世界，**只当历史读**。

## 一、一句话形状

```
图侧：inst_recipe.rs（数据收据：op id / 声明名 / 类型名 / 根 / 源文件 / body）      ← 零宏、零手写类型
stage 1 计划：px_graphs/build.rs 校验收据 → px_decls（类型化声明表）→ 算 key
              → 生成 OUT_DIR/insts_gen.rs（pub struct <T>; + impl PxOp/InstNode，事实为 const）
stage 1 执行：px build（**唯一编译入口**）→ 生成实例 crate → 编译 → target/pcg/inst/<key>.dll
stage 2：图 include! 生成物 ⇒ **用的就是 stage 1 生成的类型**；按 key 装载，签名编译期固定
```

* 接口在**编译期**（图程序），实现在**运行期**（实例库）—— 这是 R1 的来源；
  stage 1 与 stage 2 是**两个已编好的二进制**（`px.exe` / `<图>.exe`），类型不可能在运行期新生。
* 泛型实例的 key（八轴，`px_cook::inst`）：`px_inst/v1` ‖ toolchain ‖ 契约 ‖ `decl_hash` ‖ 各根名册 ‖
  接口 ‖ op id ‖ **参数源文件字节** ‖ **归一化后的体模板（`ARG` 那一份）**。改其中任何一个 ⇒ 换 key ⇒ 只重编那一条实例库。
* 今天两条实例：`cloud.coarse/band`（体积域覆盖度）、`field.remap/waves`（场域）。

## 二、命令

```
px list                                  列实例：id / 声明 / 根 / 源 / key / 有|缺
px build [--gc] [--deep] [--target]      编缺的；--gc 回收非活实例库（--deep 清生成目录，--target 清嵌套中间物）
px run <图> [--build] [--store <目录>] [图自己的参数…]
                                         两阶段：计划 →（可选编）→ 跑 target/<profile>/<图>.exe
px_render --view --scene <S.pxart> [--edit <场景配方名>]
                                         预览窗口 + **调参面板**（S9，见 44-viewer-gui.md）
                                         `--edit` 不给时按 scene 清单推配方名
tools/px.ps1 -Task list|build|gc|run     同上；图名走 -Graph <图>（不进 ValidateSet）
```
⚠ 缺实例且没给 `--build` ⇒ **非零退出并打印该跑的命令**（stage 2 绝不偷偷触发编译）。
⚠ `--store` 换的是**参数目录**（默认 `art/`），**不进键**：键跟字节走、不跟目录走。

## 三、四条铁律（改任何东西之前先读）

1. **R1**：改 `art/inst/*.rs`（或任何实现）⇒ **图程序不重编、图 exe 字节不变**；只该实例库重编。
2. **`cargo build` 绝不调 cargo、不编实例**；编译实例只能由 `px build` / `px run --build` 触发。
3. **参与算身份的 crate 只应在产品语义变化时改**：`px_fingerprint`、`px_graph_schema`、`px_*_schema`、
   `px_*_alg`、`px_*_op`。改它们**一行注释**就会换全仓节点键 ⇒ `art/anchor/hashes.txt` §三 要重登记、J1 要重跑。
   工具/编排/表（`px_cook`、`px_decls`、`px_graphs`、`art/inst`）不在圈里 ⇒ 它们的清理是免费的。
4. **先冻源码，再量 anchor**；量 R1 前先连跑到 `Compiling=0` 且无"拒绝访问 (os error 5)"（跑过 `cargo test` 后
   第一次 `cargo build` 会因特性合并重链 exe ⇒ 假红）。**撤回探针按字节精确**（untracked 文件 git 救不了，
   用 key 当 oracle 确认逐位回原值），改 untracked 源前**先记字节数**。

## 四、判据（回归时按这个清单跑）

| 判据 | 今天的值 |
|---|---|
| 两条实例 key | `cloud.coarse/band = d1c8fd369338`、`field.remap/waves = caa8318cda1b`（**都必须"有"**） |
| 生成物字节 | `OUT_DIR/insts_gen.rs` = `7740DD0C…`（宏删前删后同值） |
| 三张图产物 | planet 6/6、desert 8/8、clouds 14/14 **逐字节**（对照 `target/baseline/*.json` 记的 key；⚠ 其中 clouds 的旧键产物已不在盘上 ⇒ 需用"改动前快照"法） |
| §三 六格（`.pxart` 前 16 位） | `orbit-bare EBCD0389BEADF809` / `nolight 56E9CCBD85E4F9AB` / `shadow 6EAC1B5F1696D192` / `rings 4F716FC869DEA2CC` / `soft 2C5D222D8FC60D41` / `proxy 6391F4AE8AABD440` |
| J1 六张 PNG | `63184151909371A5` / `7BBB18CE3612D4F7` / `C03FFF3235264DD5` / `B5799E4F1649535C` / `FA20FAD37BC61EA2` / `32872F80AC867BE3` |
| 测试 | `cargo test --workspace --exclude px_render` 无 `FAILED/error`（286 passed） |

⚠ **anchor 与实例 key 都是"按 checkout"的量**：行尾（CRLF/LF）不同会让同一个 recipe 给出不同的值；
换一份 checkout 要重跑一遍登记。

⚠ **盘上有一处陈旧**（2026-09-28 实测，未修）：`art/nebula/density_volume.toml` 还是旧字段
`res_ratio`，而 `DensityParams` 只认 `res` ⇒ **`px run nebula` 在今天烘不过**
（`43-params-not-canvas.md` §2 那次改名漏了这份 toml；`origin/v2` 同样）。见 `44-viewer-gui.md` §6。

## 五、加一个泛型实例（今天的最短路径）

1. 声明：`px_*_schema/src/ops.rs` 一行 `px_op!`，并在 `px_decls/src/lib.rs` 的 `decl()` 里**加一臂**（引用真类型）；
2. 算法：rlib（`px_*_alg`）里写"图侧函数接口 + 实例库入口"；实现库（dylib）退成薄壳；
3. 图侧函数：`art/inst/<名>.rs`（纯函数、全路径 `use`、自己保证值域）；
4. 收据：`px_graphs/src/inst_recipe.rs` 加一条（`body` 里写 `ARG` 占位符，它是 key 的一轴）；
5. `px list` → `px build` → `px run <图> --build`。
   写错收据 ⇒ **`cargo build` 就 panic 并点名第几条 recipe**；体编不过 ⇒ `px build` 打出"体来自第几行 /
   参数文件 / 生成物留着"四行映射（见 `21` §7 与指南 §5.5④）。

## 六、记录地图

| 文件 | 是什么 |
|---|---|
| `44-viewer-gui.md` | **最近一轮**：预览窗口里的调参面板（改参数 → 子进程 cook → 画面重载） |
| `21-codegen-types.md` | stage 1 生成类型（recipe + `px_decls` + `insts_gen.rs`）+ 读数 |
| `20-build-graph.md` | build graph / 两阶段 / `px` driver / §192–§195 的清理与裁决 |
| `19-generic-inst.md` | 泛型实例（`px_inst!` 时代）—— key 各轴 / 握手 / 装载仍然生效 |
| `18-operator-libraries.md` | 实现库拆分与 M1/M2；那些用血换的教训仍然有效 |
| `docs/generic-op-and-graph-integration.md` | 面向使用者的指南（§5.3–§5.5 是本轮的形状） |
