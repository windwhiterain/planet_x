# CLI 读面：精简过一轮（2026-10）+ `--quiet`

> 状态：**已落地**（用户裁决：*「我要开关，并且我希望你优化一下 cli 的命令，过时的就删了，
> 而且现在直接调用 cli 不给参数居然会报错而不是弹 help」*）。
> 相关：[`agent-play.md`](../agent-play.md)（面向 agent 的手册，flag 表已同步）、
> [`sparse-history.md`](sparse-history.md)（被删的 `--notables`/`--milestones` 是那一轮的交付）、
> [`coarse-trajectory-views.md`](coarse-trajectory-views.md)（`--every`/`--traj` 那一轮）、
> [`engine-data-plane.md`](engine-data-plane.md)（「引擎给答案」的总方针）。

### `--nouns`（2026-10 新加）

`planet_x --nouns` 发**名词 → 解释**的四半语料 `{state, view, projection, control}`，与
`GET /api/schema` **同一个实现**（`agent::noun_schema_value()`）。它是**悬停弹窗的唯一文字来源**：
前端 `web/static/tip.js` 只做「拿界面上显示的名词来这里查」，自己**不认识任何领域词**。

* 语料来自 `///` 文档注释（`schemars` 收进 `description`）+ 投影的 `column_docs`。
* 用途：界面上的名词（列头/控制行标签/卡片字段名）hover 时弹它的解释；
  `g4` 有一条判据钉住「界面当名词显示的列**必须**能在这里查到词条，不许弹空框」。
* **`identity` 一块**（2026-10 加）：`{"structs": {结构体: 身份字段}, "tables": {表: 身份列}}`——
  「谁靠哪个字段认人」的**唯一声明**（结构体那半是 `model::IDENTITY`，表那半是投影的 `LAZY`）。
  Python（`_harness.identity_keys()`）、kit、前端都来问它，不许各自手抄镜像表；
  `g4` 有四条判据钉住「声明的字段**真的存在**且在一局真世界里**真的唯一**」（含视图 `key` 与
  引擎一致，前端不再各认各的）。
* 还没有词条的两类：**表达式列**（如「实力占比」，需要列上声明 `noun: "power_share"`，待做）
  与少数**注释仍是英文**的实体字段（属「用词统一」那一趟）。

## 1. 一句话

**判据 = 「这东西 `--index` 投影里有没有？」有 ⇒ CLI 上的专用 dump 开关就删掉。**
投影已经是权威读面（`play/planet_xq` 读它），CLI 每多一个 dump 开关就多一处要跟着状态结构改的
手写序列化——而 agent 的正确路径是投影，不是一堆零散开关。

## 2. 删掉的（5 个）

| 删掉 | 为什么 | 现在怎么读 |
| --- | --- | --- |
| `--traj N` | 自包含 story pack（`{schema_version,meta,story,trajectory}`）——**没有任何活的调用者**（`play/`、`web/`、测试、手册都没用它；只有早期笔记里）。它把「整局 + 规则 + 编年史」塞进一个文档，正是投影要取代的形态 | `--round N --index out/` + `planet_xq`；规则用 `--meta` |
| `--story` | 编年史**每一条投影行都带**（`chronicle` 是 `main.jsonl` 的 eager 字段，累计） | `q.facts["chronicle"].iloc[-1]` |
| `--notables [N]` | 窗口层（`State::notables`）在投影里是 `events` 表里 `salience=notable` 的行 | `q.notables()` |
| `--milestones [N]` | 里程碑层在投影里同理……**而按当前判据它恒为空**（`q.milestones()` 返回空表）——一个「永远返回 `count: 0`」的 CLI 开关就是死面 | `q.milestones()`（空表也是答案）/ `q.history()` |
| `--rounds`（`--round` 的 visible alias） | 全仓库 **0 处**使用 | `--round` |

顺带删掉的三段 main 内部代码：`run_trajectory`（43 行）、`notables_value`（24 行）、
`milestone_value`（27 行）——它们只为上面那几个开关存在。另外删掉 `agent::story_value`
（3 行 wrapper：`serde_json::to_value(&state.chronicle)`）——删掉 `--story` 之后它**全仓库
0 个调用者**；编年史由 serde 直接序列化（投影里就是），不需要这层壳。

## 3. 新开关：`--quiet`

```
planet_x --seed 42 --round 1000 --quiet --save end.json
  ⇒ 推进 1000 回合，stdout 0 字节，只落一个 0.09 MB 的档
```

* **它省的是什么**：`--round N` 的合同是**每回合吐一整份 agent 视图**（1000 回合 ≈ 40 MB）。
  只要你其实只想拿最终 state / 一层摘要，那 40 MB 就是白吐的。实测：`--round 1000` 6.3 s /
  40.3 MB → 加 `--quiet` **5.2 s / 0 MB**。
* **它不管什么**：`--digest K` 是显式请求，照旧输出（`--quiet` 只管逐回合全量那一份）；
  `--index` 本来就写文件、与它无关。
* 与它等价的旧写法是 `--digest N --save ckpt`（窗口 ≥ 总回合数 ⇒ 只出 1 行摘要），
  实测同为 5.2 s；`--quiet` 只是把「我不想要任何摘要」这件事说清楚。
* ⚠ **`--quiet` 不带 `--round`** = 什么也没干 ⇒ 仍然走 `ERR_USAGE`（那是 agent 的合同，
  不是 help）。

## 4. 裸调用：`planet_x` 打 help（不再回 `ERR_USAGE`）

```text
$ planet_x                 # 一个参数都没有 ⇒ 打 help，退出码 0
$ planet_x --seed 42       # 给了参数但没说要干什么 ⇒ 仍然 ERR_USAGE（exit 10，机器可读）
```

判据：**「一个参数都没有」= 想知道这东西能干什么**（人），**「给了参数但缺动作」= agent 用错了**
（要机器可读的错）。所以我们用的 clap（v4 derive）本身没错——`--seed` 有 `default_value`，
所以「零参数」在 clap 眼里是一个**合法**调用，落到 `main` 之后才被判成 `ERR_USAGE`。
修法是在 `main` 开头显式分流（`std::env::args_os().len() == 1` → `Cli::command().print_help()`），
而不是给 clap 加 `arg_required_else_help`（那会把退出码变成 2，且会让 `--seed 42` 也打 help）。

## 5. 行为中性怎么验的

CLI 改动碰了 `main.rs`，但**没碰 sim**。判据用仓库既有的那条：同 seed 的 digest 逐字节不变。

```text
改前：BB2EEB2BA9242519B77CE715F3B4A1045B6607E851FB28204D900089BF9B2000
      （seed 42 / 240 回合 / --digest 20，12 行）
改后：BB2EEB2B…2000   ← 逐字节相同
```

另跑了数据级三组（`play/tests/run.py all`）与 `cargo check --all-targets`（无警告）。
`--quiet` 只改「输出什么」，不改「算出什么」——它走的还是同一个 `run_rounds`，只是跳过两次 `emit`。

## 6. 没做的

* `--traj` 删了，于是 `--every K` 只剩 `--round` 一个搭档。**它没删**（`--round` 的粗读法仍是
  长轨迹唯一省 token 的读法），但它的文档现在写明「**只管 stdout 轨迹，不动 `--index` 投影**」
  ——这正是笔记 §11.1 那条实测（`--every 10` 的投影与全量一模一样）。
* `--control-plan` / `--control` / `--control-schema` / `--derived` / `--meta` / `--schema` /
  `--digest` / `--index` / `--save` / `--apply` / `--start` 都**有活的调用者**（`play/planet_x_ctl`、
  `play/tests`、web、手册），一个没删。
* 没给 CLI 引入子命令（`planet_x run/dump/...`）：现在的形态是**扁平开关 + 一个动作**，
  11 个 flag 的规模还不值得上子命令；真到要分组时再说。
