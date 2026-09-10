# play/tests —— 数据级测试（**不用测试框架**，几个脚本分组跑）

断言跑在 `planet_x` **跑出来的数据**上（`--index` 投影），不跑在 crate 内部 API 上。
方案与实测见 [`.agents/notes/test-decoupled-suite.md`](../../.agents/notes/test-decoupled-suite.md)。

## 跑

```bash
uv run --project play/planet_xq python play/tests/run.py           # 快组（内循环）
uv run --project play/planet_xq python play/tests/run.py all -j 7  # 三组全跑
uv run --project play/planet_xq python play/tests/run.py --list    # 看有哪些组
uv run --project play/planet_xq python play/tests/g3_long.py       # 单跑一个组
```

* `--bin debug` 用 `target/debug/planet_x`（重编快、跑得慢——短局用它划算）；
  `--bin <路径>` / `PLANET_X_BIN=<路径>` 指向别处的二进制。
* `--refresh` 无视缓存重跑投影（引擎行为变了但二进制指纹没变时用得上）。
* `-j N` 并行跑几个世界（默认 ~8）。

| 组 | 档 | 现在装的是什么 |
| --- | --- | --- |
| `g1_contract.py` | T0/T1（≤60 回合） | **读面契约**：同 seed 逐字节可复现；`--derived` 的 `post` ≡ `--index` 的 `view`；过程量表（`faction_process`/`city_process`）、判定表（`decisions`）、贸易两张表（`market_trades`/`haul_steps`）、输入面（`round_inputs` ≡ `pre`）逐值对账；从档投影时起点行保留真过程量；无档的 `--derived` 自报重算；`--control` 是不动点（dump→回传→逐字节相同）；中性值表里没有死路径 |
| `g2_mid.py` | T2（400 回合） | **机制不变量**：拆平的城不被旧主同回合复垦；整局里出现过装组件的活舰；编年史（RoundAt 节拍都触发/顺序单调/id 唯一/参与者具体）；没有一场战争短于疤痕承诺的回合数 |
| `g3_long.py` | T3（1000 回合 × 7 seed） | **世界健康与政治机制**：读面没有非有限的数、不进吸收态、零活城复生、经济有界、建城必须有舰、合纵连横/制裁活着、霸权叙事自洽 |

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
