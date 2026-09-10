# 用 Python 统计地编辑控制面 diff

> 状态 `[ ]`（未开工） ｜ 索引：[notes.md](../notes.md) ｜ 关联：
> `lazy-index-pandas.md`（读侧那套 Python kit）、`agent-play-friction.md` §18.1（语义指令
> 助手）、`semantic-view-api.md`（把裸 jq 降为逃生舱）、`agent-control-long-game.md`
> §1/§7/§8（预算封顶、幽灵权重、只报丢弃不报生效）、`control-live-layers.md`（写面的形状）

## 0. 目标

agent 玩长局时，「施政」这一步现在靠**手写 JSON diff**（`--apply steer.json`）。这在长局里
退化得很快：城市几十座、舰十几艘、名字还会换代，凡是「按统计量批量决定」的意图
（按维护费占比压预算、按忠诚/距离撒娱乐预算、按舰级设默认指令）都得靠人肉枚举。

要的是：**读侧用 pandas 筛、写侧产出合法 diff**，即 `planet_x_control`（名字待定）——
一个和 `play/planet_xq` 并列的 Python 套件。

## 1. 现状与缺口

* **读侧已有**：`play/planet_xq`（`--index` 投影的 lazy 索引 + pandas）。能力见
  `lazy-index-pandas.md`：`q.meta` / `q.spec(section)` / `q.<lazy表>(round=r)` /
  `q.join('<field>', round=r)` / `q.facts`。
* **缺口一：投影里没有控制面**。`src/projection.rs` 的 `LAZY` 常量只有
  `ships / cities / factions / events / bodies / settlements`——`control` 与 `scope` 都
  不在投影里。所以 Python 侧**读不到**「我现在给谁下了什么指令、预算多少」。
* **缺口二：没有 diff 构造器**。写面是 presence-aware 的多级补丁
  （`{control:[{faction_id, ship_orders[], default_ship_order, investment_budget[], …,
  buildings[]}], scope:{…}}`，见 `--control-schema`），手写容易错在：
  名字换代（`长城`→`长城2`）、`building` 下标只在城内唯一、`mode` 省略 = 写值即接管。
* **形状已经稳定**（这对本 note 是关键前提，别再改）：三态 `Inherit/Auto/Player`
  （`control-live-layers.md` §1）、读面即写面（`--control` 的模板原样回传安全）、
  `default_ship_order` 已有。

## 2. 打算怎么做

### 2.1 读控制面（两条路，先选便宜的）

* **A（推荐先做）**：Python 侧 shell out `planet_x --start <ckpt> --control`（一个 JSON
  值，读面即写面）。够用、零引擎改动。
* **B（以后再说）**：给投影加一段 control（`idx/control.jsonl`，按 round+势力）。
  好处是能看「控制面随时间怎么变」——那正是 `agent-control-long-game.md` §7 幽灵权重、
  §8「只报丢弃不报生效」这类问题的诊断数据。

### 2.2 与投影按名字 join

两边的公共键就是**唯一名字**（舰名 / 城名 / 势力名 / 天体名），`name-as-unique-key.md`
的裁决。所以：

```python
ctl = pxctl.surface(ckpt)              # 控制面（模板）
ships = q.ships(round=r)               # 投影的懒表
mine = ships.query("faction_id == '中国' and hull > 0")
ctl.ship_orders(mine.ship_id)          # 按名字取/改叶片
```

### 2.3 统计 → diff（这是本 note 的实质）

把「筛选 + 聚合」的结果落成叶片，例如：

* 按维护费占比压 `construction_budget`：`upkeep / production_value > 0.4` → 把该势力的
  造舰预算按比例降到「可养上限」以下（`--control-plan` 已经给出 `fleet_upkeep_cap` 作为
  读数，见 `agent-play-friction.md` §45）；
* 按忠诚/距离撒 `loyalty_budget`：`loyalty < 0.4 or gov_distance > 3` 的城，按人口加权
  分配（`governance-loyalty.md` 的治理模型）；
* 按舰级/角色设舰队默认（等 `control-live-layers.md` §3 的按舰级默认落地后）；
* 按矿产/面积挑 `invest_weights` 与 `build_weights`。

### 2.4 边界（别让 Python 变成"每回合重跑的假公式"）

Python 只能产出**一次性数值**。「跟着产出走」「维护费不超过产出的 X%」这类**持续意图**
必须由引擎表达（`agent-control-long-game.md` §1 的 `BudgetPatch.value` 支持
`{\"frac_of_production\": 0.3}`、`upkeep_ceiling`），Python 侧顶多在落地前当"预览计算器"。
两边各补一半，别互相假装。

## 3. 待拍板（用户尚未裁决）

* **§3.1 一次性 diff vs 可复现配方**：Python 只吐一份 `steer.json`，还是吐一份**配方**
  （脚本 + seed + round → 同一份 diff，可重放、可进 git）？
  *推荐：配方*。这个项目全程「确定性可复现」，配方即代码，也让 `play/` 里的战记能附上
  「当时凭什么这么算」。
* **§3.2 要不要给投影加 control 段**（§2.1 B）。*推荐：先不加*（shell out 够用），
  等真的出现「控制面随时间」的分析需求再加——它会让投影变重，而投影是每次 `--index`
  都要写一遍的东西。
* **§3.3 套件名字与位置**：`play/planet_x_ctl`（与 `play/planet_xq` 并列，uv 工程）？
  *推荐：并列一个新 uv 工程*，但共用 lazy 表读取（可以 import planet_xq，或抽一个
  `read_index` 公共函数）。

## 4. 落地步骤草案（等拍板后执行）

1. `play/planet_x_ctl/pyproject.toml` + `planet_x_ctl/__init__.py`：
   * `surface(ckpt_or_state) -> ControlSurface`（shell out `--control`，缓存到临时文件）；
   * `class Surface`：`.factions` / `.faction(name)` / 各叶片的 get/set（**保留 `mode` 语义**：
     `set_value(path, v)` 默认接管成 `Player`，要「只改流水」得显式传 `mode='Auto'`）；
   * `emit(path)`：产出 `${control, scope}` JSON（可直接 `--apply`）；
   * `verify(ckpt, diff)`：先 `--apply` 到一个**副本**上读回执（`NOTE_APPLY_TOOKOVER` /
     `WARN_APPLY_SKIPPED`），把「是否真的落地」在施政前就判定掉——这是对 §8 缺口的
     客户端补丁。
2. 一个示例配方 `play/exp*/recipes/*.py`（用某一局的 checkpoint 复现一两条统计施政），
   跑一遍 `--apply` + `--round` 确认没有 `warn/note`。
3. README + `agent-play.md` 里加一节「用 Python 写施政」。
4. （可选，§3.2 拍板后）投影加 control 段 + schema 更新 + `lazy-index-pandas.md` 的
   剩余项减一。
