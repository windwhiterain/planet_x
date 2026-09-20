# 23 · 观感与 graph 系统：待办与判据（滚动登记）

这一格是**持续优化**的入口：每一轮从下面两张表里各挑一条（视觉一条、系统一条），
做完把结论写进当轮笔记，并把这里的条目挪到"已做"。

⚠ 规矩（用户 2026-09-20 的测试口径）：**禁止全量测试**，只跑受影响的；不写冗余测试；
大计算量的判据转 probe。判据矩阵的写法见 `22-art-pipelines-moon-gasgiant.md` §四。

## 一、观感（按"看得见的差距"排序，目标见 `art/reference/README.md`）

| # | 差距 | 候选动作 | 怎么判 |
|---|---|---|---|
| V1 | 气态巨行星的**带边界仍偏硬、偏规则** | ① 先调材质侧 `band_contrast` / `band_gain`；② 条带场侧给 `LatBands` 加**二次谐波**或更长的相位扰动（`art/inst/latbands.rs`）；③ 试 `field.warp` 在该场上的域扭曲 | 出同机位图并排看 + 记"带边缘的过渡宽度（像素）"与值域/均值 |
| V2 | **晨昏线太窄**（参考图占球面很大一块） | 提高 `wrap`；或加一层很薄的大气壳（`atmosphere` part 已在） | 并排看明暗过渡的宽度 |
| V3 | **极区没有特征**（土星参考有极区涡旋） | `LatBands` 里对 `|direction[1]| → 1` 单独处理（极冠/涡旋） | 并排看极区，记 `direction[1] > 0.8` 那圈的值域 |
| V4 | 气态巨行星没有**环与卫星投影** | 复用 `orbit-rings` 那条路（`vocab::RING_BAND` + `ring_mesh`），卫星用一个小球 + `cast_shadow` | 并排看；阴影需要 `shadows = 1`（已有档） |
| V5 | 月球：**坑缘不够亮、没有尺度层次**（无风化层细节） | ① 坑剖面加"溅射纹"项（`crater_delta` 里已有 `rings` 的位置）；② 叠一层高频小坑（`pits` 的 octaves ↑）；③ `Palette::Moon` 的 `REGOLITH` 停靠点微调 | 并排看 + 记 `field.craters` 输出的值域/均值 |
| V6 | 月球没有**地球反照/边缘散射** | 材质侧加 `limb` 项（`gasgiant.wgsl` 里那条可以搬） | 并排看背光边缘 |

## 二、graph 系统（按"拦路"排序）

| # | 问题 | 候选动作 | 怎么判 |
|---|---|---|---|
| S1 | `Graph::finish()` 在"这份图没有任何节点写参数"时会不会写一份**空 `params.json`** | 先量：跑一张无参数的图，看 `target/pcg/<图>/params.json` 是什么 | 一次运行 + 看文件（不用测试） |
| S2 | 实例 key 吃**盘上源文件的字节** ⇒ 行尾（CRLF/LF）或编码变化就换键（这一轮踩了两次：`waves.rs`、`latbands.rs`） | 候选：key 用**规范化文本**（去掉 `\r`）算；或把行尾写进 recipe 明确声明。⚠ 这是设计点，**要先问用户** | 探针：同一份源码 CRLF/LF 两份临时副本，比较 key |
| S3 | 生成的体模板里 `ARG` 只能出现一处 ⇒ 一个实例只能实例化**一个**类型参数 | 候选：允许 `ARG` 多处 / 具名占位（`{F}`）。⚠ 设计点，**要先问用户** | 加一条 recipe 试编译（只在需要时） |
| S4 | `inst_gate::a_real_instance_cooks_end_to_end` 在实例库不在盘上时**打印一行就跳过** —— 与"任何『跳过』都是判据的敌人"冲突 | 转 probe：把端到端那一段挪进 `px_probe` 或一个 `--bin`，测试只判生成物与 key | 跑那条 probe（秒级） |
| S5 | 提示串里是否还有别处写着 `cargo run -p px_graphs --bin px` | 全仓 grep 一遍，统一走 `px_cook::inst::driver_command()` | grep + 一次实跑 |
| S6 | `px_decls` 门只数条数 ⇒ 仍不能证明"名字→类型"配对了 | 候选：`facts` 里加一条**由类型自己给的** op id，门拿它对 `px_op!` 的字符串 | 加门后跑 `px_decls` 的门（毫秒级） |

## 三、已做（这一格只留一行一条，细节在当轮笔记）

* 2026-09-20（`22`）：`field.craters`；`field.remap/latbands`；`moon` / `gasgiant` 两条管线；
  修 ① 参数进键不进计算 ② `FieldFn` 拿不到球面方向 ③ `px_decls` 双清单 ④ 帧图测试依赖本机盘；
  顺手修缺实例提示命令与 `px_shader` 的数量断言。
