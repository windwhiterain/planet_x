# play/tests —— 数据级测试（**不用测试框架**，几个脚本分组跑）

断言跑在 `planet_x` **跑出来的数据**上（`--index` 投影），不跑在 crate 内部 API 上。
方案与实测见 [`.agents/notes/test-decoupled-suite.md`](../../.agents/notes/test-decoupled-suite.md)。

## 跑

**流程 = 「build release + python 测试」**（用户裁决）：`run.py` 会先看 release 二进制是不是
比源码旧（或不存在），是就先 `cargo build --release` 再跑各组——改完 Rust 直接跑这一条即可，
不用记得先编译。已经最新就跳过（`--no-build` 强制跳过）。

```bash
uv run --project play/planet_xq python play/tests/run.py           # 快组（内循环：1 + 4）
uv run --project play/planet_xq python play/tests/run.py all -j 7  # 四组全跑（75 条判据）
uv run --project play/planet_xq python play/tests/run.py --list    # 看有哪些组
uv run --project play/planet_xq python play/tests/g3_long.py       # 单跑一个组
```

* 默认 `--bin release`：**长组的墙钟由模拟的机器码质量决定**（debug 下慢 ~4×），
  所以数据级一律走 release；`--bin debug` 只在「只跑快组、想省编译」时用，
  `--bin <路径>` / `PLANET_X_BIN=<路径>` 指向别处的二进制。
* `--refresh` 无视缓存重跑投影（引擎行为变了但二进制指纹没变时用得上）。
* `-j N` 并行跑几个世界（默认 ~8）。
* `g4_spec.py` **不用投影缓存**：它自己 `--seed 42 --round 40 --save` 起一局短局，再拿两个
  单点 dump（`--control-schema` / `--control`）做对账，整组 ~0.4 s。

| 组 | 档 | 现在装的是什么 |
| --- | --- | --- |
| `g1_contract.py` | T0/T1（≤60 回合） | **读面契约**（31 条）：同 seed 逐字节可复现；`--derived` 的 `post` ≡ `--index` 的 `view`；过程量表（`faction_process`/`city_process`）、判定表（`decisions`）、贸易两张表（`market_trades`/`haul_steps`）、输入面（`round_inputs` ≡ `pre`）逐值对账；从档投影时起点行保留真过程量；无档的 `--derived` 自报重算；`--control` 是不动点（dump→回传→逐字节相同）；中性值表里没有死路径 |
| `g2_mid.py` | T2（400 回合 × 3 seed） | **机制不变量 + 投影完备性审计**（20 条）：拆平的城不被旧主同回合复垦；整局里出现过装组件的活舰；**城的每次归属/存亡变化都有事件命名它**（实测 1933 次）/ **舰的出现有造舰事件、消失有死因事件**（337 出生 / 392 死亡）/ **一回合内同一座城不会易主两次** / **headline 逐字点到每个参与者**（32093 个实体）；编年史（节拍/顺序/唯一/参与者具体）；没有一场战争短于疤痕承诺的回合数 |
| `g3_long.py` | T3（1000 回合 × 7 seed） | **世界健康与政治机制**（10 条）：读面没有非有限的数、不进吸收态、零活城复生、经济有界、建城必须有舰、合纵连横/制裁活着、霸权叙事自洽 |
| `g4_spec.py` | T0/T1（40 回合 + 两个单点 dump） | **声明纪律**（14 条，从 `web/src/views_tests.rs` 搬来，原处留指针）：① 静态——id 唯一 / 引用完整（`use`/`use_at`/`card`/`map_ref`/`label_from`）/ `omit` 不与列重叠 / 路径表达式合文法 / `@根` 已知；② 写面对账——`--control-schema` 的 `leaves[].field` ∪ `actions[].field` ∪ `{faction_id}` **双向等于** `FactionControlPatch.properties`；③ 读面对账——**跑一局**、每片叶写一次（哨兵值）、再 `--control` 读回来，对条目字段集与「`keys` 空 ⇔ 对象」的形状；④ 认领完整性——每个叶都被 `leaf`/`action` 行认领，或在 `write_omit` 里写明理由（否则它在界面上凭空消失） |

每一条都带**防空转**判据（「这一局里真的发生过 X」），红的时候打印前几条样例 + 总处数，
样例里带 `(seed, 回合)`。

## 两件让它快的事

1. **轨迹复用**：一次运行按 `(二进制指纹, seed, 回合数, 分辨率)` 缓存进
   `target/test-fixtures/`，**所有组、所有断言共用**。指纹 = 二进制 `mtime+size` +
   `config/game.ron` 的哈希 ⇒ 代码一改自动失效，**没有手写的 golden 文件**。
   1000 回合的投影 169 MB / 13 s 一个种子，第一次要付这个钱，之后是 0。
2. **摘要**：`g2`/`g3` 从投影里抽出**每回合一行的判据表**（或一份小结），按投影缓存成
   pickle。摘要的名字里带**抽取逻辑的代码指纹**（`_harness._code_stamp`）⇒ 只改断言
   （住在 `run()` 里）命中缓存，改了抽取逻辑才重算。

缓存目录在 `target/` 下（`cargo clean` 会清掉，正是想要的），不进版本库。

## 写一条新断言

```python
ck.check("名字里带答案", not bad, "绿的时候说什么；红的时候前几条样例")
```

* 判据写进 `run()`，**数据**从摘要里拿——这样改判据不必重读 170 MB；
* 红的时候要能定位：样例里带 `(seed, 回合)`；
* **防空转**是硬要求：每条守卫都要有一句「这一局里真的发生过 X」，否则它只是在检查空表
  （本仓库的传统：宁可红，不要假装绿）。

## 纪律自己也要有量具

`play/tests/_g4_negative.py`（不是组，不进 `run.py`）把 `views.json` 与 `--control-schema`
的**副本**逐个改坏喂给 `g4_spec.run`，要求「**该红的那条红、基线全绿**」：

```bash
uv run --project play/planet_xq python play/tests/_g4_negative.py
```

2026-10 实测：**19 个注入错全部咬住**（id 重复 / 引用不存在 / 括号没闭合 / 未知 `@根` /
`omit` 与列重叠 / 叶没人认领 / 叶行指向不存在的叶 / `leaf_ui` 孤儿键 / `owner` 写错作用域 /
`write_omit` 空理由 / 声明少一片叶 / 多一片幽灵叶 / 多一个 value / 少一个 carries /
把列表叶的 keys 清空 / 把单叶的 keys 加一个 / 删掉 source 键 / 给表 layout 写 source: null / 把 `new: true` 挂到单叶上）。**一条不会红的守卫等于没有守卫**——
这份脚本就是那条判据的量具。
