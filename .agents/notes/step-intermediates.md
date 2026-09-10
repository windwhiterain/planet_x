# Step 中间量清单：36 条「算完就扔」的量

> 状态 `[~]` **B1（治理/忠诚）已落地**（`feature/step-intermediates-b1`，见 §6.1）、
> **B2（钱去哪了）已落地**（`feature/b2-money`，见 §6.3）、
> **B3（市场与运输）已落地**（`feature/b3-market`，见 §6.4）、
> **B4（战斗：中间量进事件层）已落地**（`feature/b4-combat`，见 §6.5，用户裁决 Q1 = (b)）、
> **B5（输入面 `pre`：掷出的随机数 + 判定输入）已落地（B5a）**（`feature/b5-inputs`，见 §6.6）。
> 相关：[`pre-post-unify.md`](pre-post-unify.md) §5（本篇是那一条的展开）、
> [`unified-metrics.md`](unified-metrics.md)（上一次「总结 = 步进中间量」的合并；它的「候选」第一条
> 就是本篇的 **B1**）、[`engine-data-plane.md`](engine-data-plane.md) §7.4（`pre` 面的真相 = 本篇 **B5**
> 的依据）、[`test-wall-clock.md`](test-wall-clock.md)（性能侧的另一批 P1/P2/P3，与本篇无关）
>
> **行号口径**：下表 `文件:行号` **全部在 `main` = `7e11d32` 上逐条复核过**（grep + 定点读，不是印象值）。
> 复核方式见 §9。改完代码后行号会漂——请以**量名**为准，行号只是当轮的指针。

## 0. 一句话

一次推进里，`step_*` 算了大量**只活在栈上**的中间量：为什么这座城忠诚在掉、批了 100 铁为什么只花
30、这艘船为什么一炮没发就被击沉、为什么我这单没人接。它们**算完就扔**——`RoundView` 只留下聚合后的
结果，所以玩家/agent 只能看到「忠诚掉了」，看不到「因为距离扣了 0.31、娱乐只补回 0.02」。

本篇把这 36 条盘成清单：每条带 **`文件:行号` / 量名 / 粒度 / 是否吃骰子 / 它能回答玩家的哪个问题**，
并分成 A 经济治理、B 市场运输、C 军事外交三组，最后排成 5 个批次（§6）。

## 1. 五条规矩（沿用 `pre-post-unify.md`，别另起一套）

1. **归处是 `RoundView`，不是新读面**。能塞进 `FactionRow` / `CityRow` 的就塞进去——观测与过程
   **同处一行**，这样「这个数在哪」永远只有一个答案。
2. **默认落在 `post`**：下表 36 条里 **34 条是确定性的**（纯 state/config、不用骰子），属于「这回合
   发生了什么」⇒ `post`。只有 §4 的 C7（逐舰解算顺序洗牌，**主 `Prng`**）和 C13（关系噪声，
   **主 `Prng`**）不是。
3. **吃骰子的那批属于 `pre`，而且现在还没有家**：B 组里 6 条判定走 `sim::derived_roll`
   （`role` / `route` / `gate` / `accept` / `pick` / 派工退约）——「AI 掷了什么」这件事今天的
   `pre` 面**根本记不下来**，因为 `advance` 只回 `post`。要让 `pre` 也带上这些，得改 `advance` 的
   返回（= **B5**，也是 [`engine-data-plane.md`](engine-data-plane.md) §7.4 那条缺口）。
4. **纯追加 ⇒ behavior-neutral**：只往视图里加字段、不动任何计算顺序、不消费骰子，则
   `--seed 42 --round 240 --digest 20` 的 SHA-256 **必须逐字不变**。这是每批的验收门。
5. **缺省约定照旧**：`pre`/`post` 同形，**不做** `skip_serializing_if`；「这个量在这一档不存在」
   要用 `Option` / 空 map **显式**表达，不靠「键不在」——两套缺省约定打架正是
   `pre-post-unify.md` §1 那个坑的成因。

## 2. A 组 · 经济与治理（12 条）

> **`✅B1` = 已在 B1 批落地**、**`✅B2` = 已在 B2 批落地**（字段形状与实测见 §6.1 / §6.3）；其余仍是候选。

| `文件:行号` | 量 | 粒度 | 骰子 | 它能回答什么问题 |
| --- | --- | --- | --- | --- |
| ✅B1 `sim/governance.rs:215` | `target_eff`（分项 `:213 target_base`、`:214 ent_bonus`、`:207 cap_bonus`） | 每城 | 无 | 这座城本回合的忠诚**目标值**及各分项——距离扣了多少、娱乐预算实换算成多少加成、首都人口占比 buff 多少。`view.factions[].governance_cost/coverage` 只给势力 total，所以 B1 之前「为什么这座城忠诚在掉」**没有解释面**（现在 `view.cities[].loyalty_target` 就是它） |
| ✅B1 `sim/governance.rs:209` | `ideo_penalty`（`ideology_loyalty_debuff`：`viol_mil/sci/elite/col`） | 每势力 | 无 | 「优势端思潮 vs 行为不符」扣的**全国**忠诚惩罚——为何全国忠诚一起掉（军国却不打仗、科学却不探 MOND）。⚠ 盘点时写的「`faction_ideology_debuffs`（`:129`）全仓库零调用者」**是错的**：`tests/horizon_long.rs:710` 的探针在用（每个 150 回合打印一次），漏判因为当时只 grep 了 `src/`。它不是读面的一部分，所以 B1 之前只有那条探针看得见这个数 |
| ✅B1 `sim/governance.rs:173-178` | `total_admin` vs `ent_total`（行政 vs 娱乐拆分） | 每势力 | 无 | 「钱没花在我想的地方」：娱乐预算拉满却被行政（距离 × 人口超载）吃掉。B1 之前只捕获了合计 |
| ✅B1 `sim/governance.rs:171-172` | `overload` / `scale = 1.0 + overload` | 每势力 | 无 | 人口超管理容量后**放大所有远距离城**的治理费与忠诚惩罚——「为什么治理费比上回合暴涨」 |
| ✅B2 `sim/production.rs:133` | `labor`（= `labor_ratio`；`sim/construction.rs:95` 用的是**同一把尺**） | 每城 | 无 | 人口 / 建筑用工之比，直接乘在采矿产出上——「为什么这座城产量低」= 人手不足（人口→劳力的传导点）。⚠ 已落地的是**生产那一步**用的那把（人口增长**之前**）；建造那一步另算的那把折进了 `build.<舰级>.rate`，读面不存第二份 |
| ✅B2 `sim/production.rs:91` | `is_hub` | 每城 | 无 | 产出**直进势力池**还是**先落产地货栈等船运**——「我挖出来的矿为什么用不了」。`view.cities[].production` 明确只记开采量、不分入库路径 |
| ✅B2 `sim/production.rs:121` | `housing_capacity`（+ `:103 housing_area`） | 每城 | 无 | 人口增长的**住房天花板**——「为什么人口不涨了、产出提不上去」= 住宅面积 × 生态容量封顶 |
| ✅B2 `sim/production.rs:214,216` | `short` / `frac`（`:217` 的 `.max(0.2)`；每舰 `hull_max*frac` @`:223`） | 每势力（落到每舰 hull） | 无 | 付不起维护费时舰队**按比例生锈**——「为什么我的船在掉血」。只有锈到 0 才留 `DeathCause::UpkeepShortfall` 事件，**掉血本身零记录** |
| ✅B2 `sim/construction.rs:20-21` | `inv_spent` / `con_spent`（对比 `investment`/`construction` 限额；写入点 `:218`、`:308`） | 每势力（按资源） | 无（该 step 的 rng 只被 `retool_shipyards`/`design_fleets` 消费） | 本回合**实际花掉的**投资/建造预算——「批了 100 铁为何只花 30」。限额是持久 control 叶（可见），已花量是纯局部变量（写完即弃）⇒ 读面只记已花，**相减**才是没花掉的 |
| ✅B2 `sim/construction.rs:303` | `increment`（+ `:267-272 class_rate`/`class_weight`） | 每城（按舰级） | 无 | 该舰级本回合**实得建造进度** vs 产能速率上限——造舰慢是缺钱还是缺产能；哪个舰级在抢同一笔建造预算。`class_weight` **不捕获**（它是 AI 写进 `control.build_weight` 的输入，不是丢失量） |
| ✅B1 `sim/capital.rs:47-48` | `cur_cost` / `best_cost`（+ `:45 best`） | 每势力 | 无 | **迁都判据数字**：新旧首都的「总治理距离成本」各是多少、候选城是谁。事件 `CapitalRelocated` 只带 `reason` 字符串，**不带数字** |
| ✅B1 `sim/capital.rs:60-61` | `old_share` / `loyalty_cost` | 每势力 | 无 | 迁都当回合对**全国每座城**的忠诚扣减及其来源（旧首都人口占比）——「为什么迁都以后忠诚集体掉了一截」 |

## 3. B 组 · 市场与运输（12 条）

| `文件:行号` | 量 | 粒度 | 骰子 | 它能回答什么问题 |
| --- | --- | --- | --- | --- |
| `sim/market.rs:239-242` | `p_eff = price × (rel_mult + freight_rate)`，分解项 `dist_au` / `depth` / `mond_extra` / `rel_mult` | 每次结算（势力对 × 资源） | 无 | 为什么是这个价：向敌人买贵、向朋友买便宜、运得远又贵、过异常带再贵；价高过购买力时这笔**干脆不成交** |
| `sim/market.rs:254-261` | `loss` / `lost_units` / `reliable`（= `is_mond_master`） | 每次结算（势力对 × 资源） | 无（确定性比例） | 为什么我的货少了：非 master 走异常带按深度丢货，`settled` 只记**收到**的那份 |
| `sim/market.rs:180-184, 223-225` | `listed_value`（购买力）+ `buyers` 购买力降序序位 + `spendable` / `max_purchase` | 每势力（每回合结算） | 无 | 为什么有货在卖我却没买到：买家按购买力排队（**钱多先买**，同额按名字），排后面/可变现富余不够就买不到 |
| `sim/market.rs:58-79`（用点 `:214`） | `trade_block_cause` 的 `"war"` / `"cold"` / `"coalition"` | 势力对 | 无 | 为什么它不卖我：开战 / 关系冷过 `embargo_relation` / 联盟封锁霸权——**三档原因对策不同** |
| `sim/haul.rs:49-59, 224-236, 276`（用点 `autocontrol/tactics.rs:487`、`sim/military.rs:95`） | `HaulStep` 的变体与 `units`（`military.rs:95` **整个返回值丢弃**） | 每舰（每回合） | 无 | 为什么这趟货没运回来：本回合是「停在**空**货栈干等」「还在路上」还是「装了 N 件」。`Waiting`/`EnRoute` **不落 State 也不发事件** |
| `autocontrol/contract.rs:131-134`（+ `:117-128` / `:82-84` / `:94-104`；闸门 `:185-199`） | `accept_chance` / `decision_value`（`reward`、`p_ok`、`d_rep`）/ `eligibility` / `available_throughput` | 每次结算（单 × 受雇方） | 概率本身无骰；闸门走 `derived_roll`（`gate`/`accept`/`pick`） | 为什么我这单没人接、或我没被请：没看见 / 不想接 / 一条船都派不出 / 抽签输 |
| `autocontrol/contract.rs:333, 337`（+ `:252-261` / `:276-286` / `:323` / `:351`） | `hired_throughput` / `shortfall`、`own_ship_balance` 的 `(spare, deficit)`、`lend`、`quit` | 每次结算（合同）+ 每势力每回合派工 | `derived_roll`（`派工{id}` / `退约{id}`） | 为什么我的单上一直没船来；为什么我的船被抽回去了（自家缺 k 条船 ⇒ 退最不划算的单） |
| `autocontrol/freight.rs:484-524` | `capacity_ledger` 的 `(need, own, hired, uncovered)`（`:535` 汇总 `uncovered`） | 每次结算（货栈/天体）× 每势力每回合 | 间接（内部 `serving_freighters` → `should_be_freighter` 取 `"role"`） | 为什么这处积压挂不出单（缺口 0）、或挂了多少 = 要求运力 − 自有期望份额 − 已雇运力 |
| `sim/mond.rs:43-62, 69-71`（用点 `sim/geometry.rs:32`） | `mond_drift` 的偏移量 + `nav_roll` 的 `roll` | 每舰每次尝试 | `derived_roll`（**空盐 = 导航档**） | 为什么深处目标一直没到：本回合坐标被切向平移 `depth × drift_per_au × roll^shape`，`roll≈0` 才指哪打哪 |
| `autocontrol/freight.rs:187-259`（`p` `:252`、`gap` `:246`、票 `:219-242`） | `should_be_freighter` 的 `quota` / `gap` / 票 / 入伙退伍概率 `p` | 每舰（每回合定编，每势力） | `derived_roll`（`"role"`） | 为什么这艘舰改行了 / 为什么没人肯跑运输：目标头数 = 需求 × 思潮倾向，缺口按效率票抽签 |
| `autocontrol/freight.rs:346-360`（抽签 `:351`） | `route_for` 的 `cands`（各货栈积压）/ `total` / 抽签值 `x` | 每舰（择线时一次） | `derived_roll`（`"route"`） | 为什么这艘船去了那处货栈（而积压更大的地方没人管）：按积压占比掷一次；**续用旧线时不掷** |
| `sim/mond.rs:122-133`（用点 `model/contract.rs:390`） | `mond_arrival_chance(config, depth)` 的单次尝试成功率 `p` | 每舰 / 每条线 | 无（纯 state） | 还要试几个回合才进得去：`p` 恒 > 0、随深度单调降。它已被折进合同 interval/capacity，但**舰自身的导航 p 无人记** |

`step_contracts`（`sim/market.rs:345-360`）本身**无候选**。

## 4. C 组 · 军事与外交（12 条，+1 条骰子补充）

| `文件:行号` | 量 | 粒度 | 骰子 | 它能回答什么问题 |
| --- | --- | --- | --- | --- |
| `sim/military.rs:297` | `hit = hit_factor(w.tracking, tspeed)`（含被规避比例 `evade`） | 每次结算（攻击舰 × 目标舰，势力对） | 无（纯函数） | 为什么这一发几乎没伤害 / 像没打中——目标速度对武器追踪的**确定性**规避折减（不是运气）。`Attack` 事件只有折减后总伤害，分解丢失 |
| `autocontrol/tactics.rs:181` | `build_fire_plan`（逐发 `(武器下标, 目标)` + 回合内自更新本地 `hist`，`:134` 起手取 `attack_hist`） | 每次结算（逐舰，可细到舰对） | 无（`weapon_noise` 是 `(weapon.seed, 目标名)` 哈希噪声，不吃主 `Prng` 也不用 `derived_roll`） | 为什么这门炮打它而不是那艘残血舰 / 火力为什么被摊到多个目标。`decisions.ships.target` 只记**主**目标，**玩家舰连主目标都不记** |
| `autocontrol/tactics.rs:79` | `doctrine_weight` 分项（`dist_score` / `weapon_counter` / `weapon_noise` + `W_TEMPER·-temper·ln(威慑比)` + `fire_spread·recency`） | 每次结算（舰对） | 无（同上，哈希噪声） | 为什么 AI 挑了这个目标——是距离、克制、理智/热血，还是「最近打过它」把它推上去的 |
| `sim/military.rs:320` | `armor_soak`（连带 `absorbed` / `soak` / `:325 hull_pen`） | 每次结算（每舰为目标时） | 无 | 为什么这条战列舰/带盾舰难啃——硬度按反比例减伤吃掉的船体伤害、护盾先吸掉的那一层 |
| `sim/military.rs:301` | `pd = tpanel.intercept + cluster_pd_cover(...)`（后者定义 `:362`） | 每次结算（每舰为目标时，含邻近友舰） | 无 | 为什么我的导弹齐射被吃光——目标自身点防 + 邻近友舰防空屏护的**线性**拦截量（舰沉后无法重算） |
| `sim/ideology.rs:83` | `targets`（每势力 4 轴 target；驱动量 `mil` `:87`、`sci_ships` `:89`、`tech_cities` `:90`、`net`、`pca` `:123`） | 每回合（每势力） | 无 | 为什么思潮往那边偏——这条轴本回合信号值是多少、由什么产生（击沉/失城、MOND 到访 vs 开采、经济净流、人均面积） |
| `sim/military.rs:13` | `order`（`:15 rng.range(i+1)` 洗出的逐舰解算顺序） | 每回合（世界级逐舰顺序） | **主 `Prng`**（每回合一次洗牌，非 `derived_roll`） | 为什么这艘舰一炮未发就被击沉——互杀时它排在击沉它的舰**之后**（`hull<=0` 跳过、目标已毁的 plan 作废） |
| `sim/relations.rs:47` | `war_scar_floor`（应用处 `v.min(floor)`：`:92` / `:204`） | 每回合（每势力对） | 无 | 为什么刚打完的对手当回合不能言和——记恨地板此刻压到多少、还剩几回合变淡 |
| `sim/relations.rs:157` | `aff`（静息亲和 = `affinity_floor/span·\|Δalignment\|` @`:157` + `ideology_affinity_span·(2·sim−1)` @`:162`） | 每回合（每势力对） | 无（本回合的 rng 扰动是**另一条**，见下） | 为什么这两国关系一路朝战争漂（或相反）——它正朝哪个值靠拢、该值由阵营亲缘 + 思潮相似度合成 |
| `sim/ships.rs:100` | `deterrence`（= `ship_power` + `deterrence_radius` 内同势力战力；`ship_power` @`:117`） | 每次结算 / 每次判定（每舰，含友舰叠加） | 无 | 为什么敌人专挑它打 / 它为什么怂——`temper` 轴比的就是双方威慑比（`style_retunes` 只记派生比 `power`，索敌层没记） |
| `autocontrol/tactics.rs:252` | `kiting_dest`（含 `awareness` / `desired_r`；`sim/haul.rs:270` 同用，覆盖点在 `tactics.rs:535`） | 每次结算（每舰） | 无 | 为什么我下的 `Move`/`Haul` 命令**没照办**、舰自己回缩或贴上去——软目标覆盖了玩家给的坐标，而**玩家舰这条覆盖没有任何记录**（移动无事件；AI 舰已有 `decisions.ships.destination`） |
| `sim/power.rs:231` | `now_at_war.difference(&was_at_war)`（集体安全触发 + 应用的 `collective_defense_delta` @`:235`；`was_at_war` @`:202`） | 每回合（每势力对：霸权 × 各弱者） | 无 | 为什么霸权一动手、其余弱者关系当回合**集体**骤降——「攻其一方 = 与全体为敌」的那次级联 |
| `sim/relations.rs:180` | `rel += rng.range_f64(-d.noise, d.noise)`（**第 37 条**，盘点时只在总结里提过，未进上表） | 每回合（每势力对） | **主 `Prng`** | 关系为什么**无端抖了一下**。与 C7 同属 `pre` |

`power.rs` 的霸权/联盟/制裁三项判定**已被 `view.hegemon` / `coalition_members` / `sanctioned` 捕获**，
故只报上面那条级联。

## 5. 为什么要捕获：按「还能不能拿到」分三档

盘点时对 36 条做了一次「事后再去 state 里找，找得到吗」的判断（**是判断，不是已验证的结论**）：

* **甲 · 本回合内就没了 ⇒ 不捕获就永远读不到**（沉船、位置已变、局部变量写完即弃）：
  A 组 `target_eff` 分项 / `inv_spent` / `con_spent` / `increment` / `cur_cost` / `best_cost` / `old_share` /
  `short`；B 组 `p_eff` / `loss` / `HaulStep` / `capacity_ledger`；C 组 `hit` / `build_fire_plan` /
  `doctrine_weight` / `armor_soak` / `pd`。**这批是捕获的主要理由**——它们是「唯一真相」而不是
  「另一个副本」。（A 组这八条**已全部落地**：B1 收前五条、B2 收后三条。）
* **乙 · 不持久、但可从回合末 state 重算 ⇒ 捕获的收益是「与引擎逐字一致」**（不必额外维护一条公式）：
  A 组 `ideo_penalty` / 行政娱乐拆分 / `overload` / `labor` / `is_hub` / `housing_capacity`；
  B 组购买力序位 / 禁运三档 / 合同四闸门 / 单次导航成功率；C 组思潮 target / `war_scar_floor` / `aff` /
  `deterrence` / `kiting_dest` / 集体安全级联。
  照 [`unified-metrics.md`](unified-metrics.md) 的老规矩（**总结不该被独立重算一遍**），这批也归引擎。
  ⚠ **B2/B3 实测发现这批里有几条连「重算」都不成立**：`labor` 取的是人口增长**之前**的人口、
  `is_hub` 取决于**本回合中途**的迁都/易主、`capacity_ledger` 与 `haul_gap` 是**挂单那一步**
  （回合中段，船还没动、货还没装卸）算的 ⇒ 回合末重算会给出另一个数。所以它们其实是**甲**。
* **丙 · 吃骰子 ⇒ 读面看不到「掷了什么」**：C7（主 `Prng` 洗牌）、C13（主 `Prng` 噪声），
  加上 B 组 6 条 `derived_roll` 判定（`role` / `route` / `gate` / `accept` / `pick` / 派工退约）。
  **这一批必须进 `pre` 面**（B5）。

## 6. 批次（建议的开工顺序，每批独立可合并）

| 批 | 内容 | 为什么这个顺序 | 是否要动 `advance` 返回值 |
| --- | --- | --- | --- |
| ✅ **B1** 治理/忠诚（**已落地**，见 §6.1） | A 组 `target_eff` 分项、`ideo_penalty`、行政 vs 娱乐拆分、`overload`、迁都判据（`cur_cost`/`best_cost` + `old_share`/`loyalty_cost`） | `unified-metrics.md` 候选第一条；「帝国为何要崩」的预警面；全是确定性、粒度天然对齐「每城一行 / 每势力一行」 | 否 |
| ✅ **B2** 钱去哪了（**已落地**，见 §6.3） | A 组 `inv_spent`/`con_spent`、`increment`/`class_rate`、生锈 `frac`、`labor`、`housing_capacity`、`is_hub` | 回答「批了为什么没花」；`FactionRow`/`CityRow` 各加几列即可 | 否 |
| ✅ **B3** 市场与运输（**已落地**，见 §6.4） | B 组 12 条里的**确定性 6 条**（`p_eff` 分解、丢货、购买力序位、禁运三档、`HaulStep`、`capacity_ledger`） | `HaulStep` 是「货为什么没运回来」的唯一入口（连事件都没有）；`capacity_ledger` 补上「挂单数量从哪来」 | 否 |
| ✅ **B4** 战斗（**已落地**，见 §6.5） | C 组 `hit`、`armor_soak`、`pd`、`deterrence` + 索敌计划（`build_fire_plan` / `doctrine_weight`） | 玩家最想要的一批（「为什么我打不中」），**但粒度最麻烦**——见 §7 Q1 | 否 |
| 🚧 **B5** `pre` 面（**B5a 已落地**，见 §6.6；B5b/B5c 待接） | C7 洗牌顺序、C13 关系噪声、`derived_roll` 家族（**实测 ~18 处**，不是 6 条）、判定时看到的候选池 | 唯一一批**必须**从回合中段捕获（`advance_round` 把输入面交出来）；面的分工已在用户裁决下定死 | 否（`advance` 保留原签名） |

每批的验收门（缺一不可）：

1. `--seed 42 --round 240 --digest 20` 的 SHA-256 **逐字不变**（取行口径见 §9）；
2. `cargo nextest run -P full` 全绿（快档 ~2 s、中档 ~8 s、全档 ~27 s
   ——`[profile.test] opt-level=2` 之后，见 [`test-wall-clock.md`](test-wall-clock.md) §0）；
3. 读面契约测试（`tests/projection_derived.rs`）加一条：新列在 `pre`/`post` 两档都存在且类型一致；
4. Python kit（`play/planet_xq`）与 web 信息树能**泛化**读出（这两处本来就是 generic 渲染，通常零改动）。

### 6.1 B1 落地记录（`feature/step-intermediates-b1`，全部实测）

读面（`RoundView`，`pre`/`post` 同形）：

| 位置 | 新字段 | 缺省（`pre` 里 / 那一步没跑） |
| --- | --- | --- |
| `view.cities[<城>].loyalty_target` | `distance` / `entertainment` / `capital_share` / `ideology_penalty` / `effective`（= 四项之和 clamp 到 0..1） | 全 0 |
| `view.factions[<势力>]` | `governance_admin` / `governance_entertainment` / `governance_scale` / `ideology_loyalty_penalty` | 0 / 0 / **1.0** / 0（倍率的中性缺省是 1.0，同 `governance_coverage` 那条约定：缺的是「没有账」，不是「治理能力归零」） |
| `view.factions[<势力>].capital` | `reviewed` / `candidate` / `current_cost` / `candidate_cost` / `relocated_from` / `relocated_to` / `relocate_loyalty_cost` | `reviewed=false` + `Option` 全 `None`（用 `Option` 表达「**没算**」，不拿 0 冒充成本 0） |

引擎内部：`RoundSink` 新增 `city_loyalty`（每城）与 `capital`（每势力）两张 map；`GovernanceFlow` 扩成
`total / coverage / admin / entertainment / scale / ideology_penalty`（一个势力**一条**记录，不另开平行
map）；`step_capital` 因此多了 `flow: &mut RoundSink` 形参。

tidy 表：`idx/faction_process.jsonl` += 4 列（`governance_admin` / `governance_entertainment` /
`governance_scale` / `ideology_loyalty_penalty`）；`idx/city_process.jsonl` += 5 列
（`loyalty_target_effective` / `_distance` / `_entertainment` / `_capital_share` / `_ideology_penalty`）。
**迁都判据不进表**（它稀疏——只有评估回合才有数），只在 view 里；kit 用
`q.view_economy(r, f)["capital"]` 读。两张表的 `columns` 与 `column_docs` 都补了。

`SCHEMA_VERSION` **14 → 15**：又是「只动派生读面、`State` 字段一个没动」⇒ `migrate` 加一档 `14 =>` 推号。

| 判据 | 结果 |
| --- | --- |
| 行为中性：`--seed 42 --round 240 --digest 20` 的 SHA-256 | **`657F2DC9…66665`，逐字不变**（就是 B1 之前那条基线） |
| `cargo nextest run -P full`（工作区） | **192 passed / 0 failed / 25 skipped，29.3 s** |
| 点名 13 条（3 条新用例 + 两条读面契约 + 迁都三条） | 13/13 通过 |
| 新用例 | `loyalty_target_decomposes_the_loyalty_equation`（勾稽 / 全国同值 / 距离项单调 / 折进视图后逐值不变）、`governance_cost_splits_into_admin_and_entertainment`（`total = (admin + ent) × 制裁倍率`、有活城必有行政开销）、`pre_view_has_neutral_b1_defaults`（缺省值说话算话） |
| kit 端到端（`--seed 7 --round 30` 的真投影跑 `play/planet_xq/demo.py`） | 中国 r30：治理总开销 **6.0 = (行政 1.0 + 娱乐 5.0) × 1.0**、覆盖率 1.0、超载倍率 1.0、思潮惩罚 **0.0765**；`q.view_loyalty(30, "中国")` 五行城的四项分项齐全，demo 里的勾稽断言（`effective = clamp(四项和)`）通过 |
| web | 信息树本来就是 generic 渲染整份 `RoundView` ⇒ **零改动**（新增字段自动可见） |

一个**顺手做的判断**（行为中性，值得记下来）：AI 迁都评估原本只在「候选 ≠ 现首都」时才算那两笔
成本，B1 改成**评估回合一律算**——否则「为什么没迁」恰好是唯一读不到的那种情况。两个函数都只读
城市位置与人口，无副作用、不消费骰子。

### 6.2 形状修订（`feature/capital-decisions`）

B1 落地后量了字节，发现两处不合本项目自己的规矩，于是把它俩收掉了（细节与裁决理由见
[`dense-face-sparse-store.md`](dense-face-sparse-store.md) §8——那里也记了「**为什么不做**
通用稀疏层」）：

| 原形状 | 现形状 | 为什么 |
| --- | --- | --- |
| `view.factions[<>].capital{7 字段}`（平时 6 个 `null`，1350 B/行，而它 11/12 回合无事发生） | **`view.decisions.capital[]`**（稀疏数组，一行一条首都判定；缺席 = 既没评估也没迁） | 「大部分回合无事发生」的判定归判定数组（同 `ShipDecision` 的 `hold`） |
| `view.cities[<>].loyalty_target` 里的 `capital_share` / `ideology_penalty`（按势力算一次，却在每座城抄一遍） | 城行只留 `distance`/`entertainment`/`effective`；那两项**只在势力行**（`capital_loyalty_bonus` / `ideology_loyalty_penalty`） | 「同一个数只有一个位置」 |

读面的忠诚目标式因此变成：

```text
effective = clamp(distance + entertainment
                  + factions[<势力>].capital_loyalty_bonus
                  - factions[<势力>].ideology_loyalty_penalty, 0, 1)
```

实测（同口径 `--seed 7 --round 6`）：`main.jsonl` **18127 → 15652 B/行**（B1 的净新增
5338 → 2485 B/行）；`capital` 那项在 30 回合整段里 40500 → **4185 B**；城侧 `loyalty_target`
3653 → **1908 B/行**。`SCHEMA_VERSION` 15 → 16；`idx/decisions.jsonl` 多一类 `kind="capital"` 行；
`faction_process` 多一列 `capital_loyalty_bonus`，`city_process` 少两列。
验收：digest **仍逐字不变**、全档 **198 绿**、kit 端到端勾稽通过。

### 6.3 B2 落地记录（`feature/b2-money`，全部实测）

**读面新增 8 个字段**（`RoundView`，`pre`/`post` 同形；中性值全部在 `model::neutral` 声明）：

| 位置 | 新字段 | 中性值 | 它回答什么 |
| --- | --- | --- | --- |
| `factions[]` | `investment_spent` / `construction_spent` | `{}` | 本回合**真花掉**的投资/建造预算，按资源（写完即弃的局部变量） |
| `factions[]` | `upkeep_unpaid` | `0.0` | 付不起的那部分维护费（= 生锈的分子） |
| `factions[]` | `fleet_rust` | `0.0` | 每艘舰被锈掉的**船体比例**（`hull_max × 它`；锈到 0 才发事件） |
| `cities[]` | `labor` | **`1.0`** | 用工系数（人口 ÷ 用工需求）——产量为什么低 |
| `cities[]` | `housing_capacity` | `0.0` | 住房天花板（住宅面积 × 生态容量）——人口为什么不涨 |
| `cities[]` | `is_hub` | `false` | 本城天体是不是首都集散地（产出直进势力池 vs 先落产地货栈） |
| `cities[]` | `build`（`{舰级: {rate, increment}}`） | `{}` | 造舰是**缺钱**还是**缺产能** |

**一条形状裁决（与 B1 的「同一个数只有一个位置」同源）**：「**批了多少**」不进读面。
限额是控制面的**持久叶**（`control` 的 `investment_budget`/`construction_budget`，`write_budget`
每回合把当回合用的额度写回去、已摊在 `derived.control` 表里），所以读面只记「已花」，
**两者相减** = 「批了却没花掉」。存第三个数（余额）就是第二个副本。这条 join 写进了
`schema.json` 的列说明，也有测试钉着（`spent_never_exceeds_the_batch_and_the_gap_is_the_unspent_part`）。

**中性值里最容易被改错的两个**（都进了 `PROCESS_PATHS` 守卫）：
用工系数的中性值是 **1.0**（不缺人手）而不是 0（那会被读成「全城没人上工」）；
`is_hub` 在 `pre` 里是 `false`（这个月的入库路径还没定），**不是**「它不是首都」——
要读「此刻谁是集散地」得拿 `control` 的 `capital` 叶比 `body_id`。

**体积账**（同口径 `--seed 7 --round 6`，7 行，`main.jsonl` 文件字节；字段级含键名）：

| 量 | B1（`29cb1d1`） | B2 | 说明 |
| --- | --- | --- | --- |
| `main.jsonl` | 15652 B/行 | **18662 B/行**（+19.2%） | 整行含转义 |
| B2 新增合计 | — | **+2993 B/行** | 城侧 +2061、势力侧 +932 |
| `cities[].build` | — | 920 B/行（41.8 B/值，69% 有值） | B2 里最贵的一项（逐城 × 舰级两个全精度浮点） |
| `cities[].housing_capacity` | — | 554 B/行 | |
| `factions[].construction_spent` | — | 352 B/行 | |
| `cities[].is_hub` | — | 323 B/行（67% 是 `false`） | |
| `cities[].labor` | — | 264 B/行 | |
| `factions[].investment_spent` | — | 229 B/行 | |
| `factions[].upkeep_unpaid` / `fleet_rust` | — | 189 / 162 B/行 | |

**新读面顺手量出来的一件事**（不是 B2 引入的，是它第一次看得见）：`--seed 7` 的 r30 上，
中国的**投资预算 41 铁批了、花 0**，而**造舰预算只剩 0.02 铁**——`read_budget` 的
「维护费保留」（`upkeep_reserve_mult`）把库存吃到只剩零头，于是 4 个建造区里 3 个是
`idle`（有产能、一分钱没批到）、1 个是 `money`（批到的那点钱只够 0.0057 进度）。
这正是 B2 想让玩家看见的那一格：「我的船坞为什么空转」= 钱被舰队维护费的预留吃掉了。
（**行为一行没改**——digest 逐字不变；要改的是平衡，不是读面。）

验收：digest `--seed 42 --round 240 --digest 20` **仍逐字不变**（纯追加 ⇒ 行为中性）；
`cargo nextest run -P full` **203 绿 / 0 红 / 25 skipped**（B1 时 198）；新增 5 条 B2 单测
（`src/tests/sim/spending.rs`）+ 两条投影契约列（跨进程逐值一致，且整局防空转）；
`SCHEMA_VERSION` 16 → 17（迁移档只推号，`State` 字段没动）；kit 新增 `view_spending()`
（批−花−没花 × 逐资源、造舰瓶颈判据、掉血），端到端 demo 勾稽通过。

**B2 之后的重新测量**（与「要不要做通用稀疏层」有关）落在
[`dense-face-sparse-store.md`](dense-face-sparse-store.md) §9：那里把「中性值能省多少」从
印象值换成了实测上界（**占整个 view 的 25%、3826 B/行**），并据此改写了触发条件。

### 6.4 B3 落地记录（`feature/b3-market`，全部实测）

**读面新增 5 片**（`RoundView`；中性值全部在 `model::neutral` 声明，`PROCESS_PATHS` 守卫跟着扩）：

| 位置 | 新东西 | 中性值 | 它回答什么 |
| --- | --- | --- | --- |
| `view.market_trades`（**数组**） | 一笔成交一行：`buyer`/`seller`/`moved`/`dist_au`/`depth`/`mond_extra`/`freight_rate`/`rel_mult`/`mastery`/`loss` | `[]` | 「**为什么是这个价**」（分解式）与「**我买到的货为什么少了**」（丢货率） |
| `view.haul_steps`（**map：舰名 → 动作**） | `HaulStep` 四档：`loaded`/`delivered`/`waiting`/`en_route` | `{}` | 「**这趟货为什么没运回来**」——`waiting`/`en_route` **既不落 state 也不发事件**，此前零读法 |
| `factions[].purchasing_power` + `market_rank` | 结算那一刻的可出口富余价值 + **买方队列名次** | `0.0` / **`null`** | 「**有货在卖我却没买到**」= 钱多的人先挑，我排第几 |
| `factions[].freight_gap`（map：天体 → 账） | `need`/`own`/`hired`/`uncovered`（**挂单用的同一本账**） | `{}` | 「**哪处货栈在积压、缺口多少**」——势力级只有 `haul_gap` 一个比值 |
| `factions[].trade_blocked_by`（**升级**） | 从 `usize` 计数 → `{对方势力: war｜cold｜coalition}` | `{}` | 「**谁不卖我、为什么**」——三档对策完全不同，计数答不了 |

**三条形状裁决**（都写进了 `schema.json` 的列说明，并被用例钉住）：

1. **成交清单的粒度是「一对（买方 × 卖方）一行」，不是「一对 × 一资源一行」**。那一行里除了
   `moved`，其余每个数**只由这一对决定**（距离/异常带深度/关系倍率都与买哪种矿无关），
   所以每种矿的成交价就是 `view.market_price[资源] × (rel_mult + freight_rate)`——而 `p` 本来就
   在 `market_price` 里。按「一对 × 资源」展开会把同一组分解数抄 N 遍（贵且容易漂）。
2. **`moved` 的语义是「卖方交出的量」**，买方收到的是 `moved × (1 − loss)`——**后者**才是
   `view.market_settled` 记的那份。这条恒等式有用例逐资源对账（漏一行/重复计/写反语义都会红）。
3. **`haul_steps` 两条执行路径都写**：AI 的 `ai_ship_turn` 与**玩家指令**的 `step_military`
   （玩家舰不产生 `decisions.ships` 行，所以不能塞进判定表——它是「结算事实」不是「AI 判定」）。
   `HaulStep` 的定义因此**搬进了 `model`**（`model/haul.rs`）：`model` 不许依赖 `sim`，
   而它同时是引擎内部类型与读面类型 ⇒ 定义只留一份（`sim::haul` 只做转出）。

**体积账**（同口径 `--seed 7 --round 30`，31 行，逐子树实测；键名与分隔符都算。
复算脚本 `C:\resource\px_b3_bytes.py` **不跨版本比总额**，只量 B3 自己那几个子树——合并后的
轨迹与 B2 那棵树不同，跨版本比总额会把轨迹差异混进来）：

| 项 | B/行 | 条目/行 |
| --- | --- | --- |
| `view.market_trades` | 631.5 | 3.0 笔 |
| `view.haul_steps` | 305.3 | 6.6 舰 |
| `factions[].freight_gap` | 652.1 | 7.3 处货栈 |
| `factions[].trade_blocked_by`（升级净增） | +333.4 | 522.4 − 189.0（旧计数） |
| `factions[].purchasing_power` / `market_rank` | 268.9 / 144.9 | |
| **B3 合计净增** | **+2336 B/行** | view 的 11.8% |

**新读面顺手量出来的两件事**（都不是 B3 引入的，是它第一次看得见）：

* **同一天体上的两家之间没有运费**：`dist_au = 0 ⇒ freight_rate = 0`（地球上五座城属于不同
  势力，它们之间当然不该有星际运费）。所以「运费分解」要跨回合看才看得到非零项——用例里
  专门留了一档断言，免得守卫在「全是零运费」时静默空转。
* **雇主眼里的「自有运力」会把被雇走的船扣掉**（`serving_freighters` 的既定设计），于是会出现
  `freight_gap` 里 `own = 0`、而同一回合这个势力自己的船**明明在跑运输**（跑的是**别人的**线）。
  这不是矛盾，是分工；读面第一次把这两半摆在一起。⚠ 顺带发现：这种「一个能派的船都没有」的
  `own` 会写成 **`-0.0`**（Rust 对**空迭代器**求和从 `-0.0` 起折的符号位），数值上等于 0——
  判空请用 `== 0.0`，文档里也标了。

验收：digest `--seed 42 --round 240 --digest 20` **逐字不变**（= `81A197…1811`，与合并后的
`main` 相同 ⇒ 纯追加、行为中性）；`cargo nextest run -P full` **225 绿 / 0 红 / 28 skipped**
（B2 时 218，新增 6 条 B3 单测）；两张新派生表进 `DERIVED`（`idx/market_trades.jsonl`、
`idx/haul_steps.jsonl`，**不做 r2 舍入**——过程量表是视图的平铺版，两个读面必须逐值相同），
跨进程逐值一致 + 整局防空转都有用例；`SCHEMA_VERSION` 18 → 19（仍只动派生读面）；
kit 新增 `market_trades()` / `haul_steps()` / `view_trade()` / `view_freight()`，
`view_economy` 补购买力/名次/`haul_gap`/禁运名单，demo 端到端勾稽通过（勾稽式：
`price_mult == rel_mult + freight_rate`）。

### 6.5 B4 落地记录（`feature/b4-combat`，Q1 裁决 = **(b) 进事件层**）

**用户裁决**：Q1 选 **(b)**——战斗的逐发中间量放**事件层**，不占每行内联的 `RoundView`。
加一句澄清（用户原话：*「kit 的速度不重要，我是说游戏中查询 event」*）：这条裁决看的是**引擎内**
的读取与体积，不是 Python 侧的分析速度。

**落点形状**：**不新增 variant**，而是给已有的 `GameEvent::Attack` 加 `shots: Vec<Shot>`。

| 为什么 | 说明 |
| --- | --- |
| `Attack` 已经是「一条 =（攻击舰 × 目标）聚合伤害」 | 逐发是它的**下钻**，不是另一种事件；标题、`magnitude`、参与方槽位全都不用动 |
| 事件行有 **`data` 对象列** | 变体专属载荷只有一个出口（`EventRow.data`），投影表自动多出 `data.shots`，零新表 |
| 不新增 variant ⇒ 不动 `salience`/`headline`/`participants` 三处穷尽 match | 少三处「编译器逼你写」的地方，也就少三处漂移点 |
| 事件在 `main.jsonl` 里**只有 id**（`event_ids`，约 5.5–7 B/条） | 所以这一批**一个字节都没进轨迹行**——这正是 (b) 相对 (a) 的全部价值 |

**`Shot` 一条 = 一件武器的一发**，两类数同处一条：**选择输入**（`score_basic` / `score_temper`
/ `score_spread`，总分 = `(basic + temper) × spread`——第三项是**乘数**，文档里写清了）与
**结算分解**（`hit` / `def_mult` / `pd` + `pd_absorbed` / `absorbed` + `soak` / `armor_soak` /
`hull_pen` / `damage` / `killed` / `skipped`）。分开放只会让「选它的理由」和「打出来的结果」
两边漂。

**一处必须一起改的判据（B4 唯一的非纯追加点）**：**放宽了发事件的闸**——从「总伤害 > 0 才发」
改成「真朝一个**活**目标打过一发就发」。理由：`pd` 是**线性**拦截，被吃光时 `damage = 0`，
而「我的导弹齐射为什么全被拦下了」（C5 的门面问题）在旧规则下**一条事件都不留**。
⚠ 这会让 `step_diplomacy` 的「本回合谁和谁交火」（它由 `Attack`/`Siege` **反推**）把「打了一发
被拦光的空炮」也算成交火 ⇒ **战争疲劳会被空炮取消**。所以同批给那条判据加了显式的
`damage > 1e-9` 闸（`relations.rs`），关系调整也照旧只认真伤害 ⇒ **世界行为逐字不变**：

```text
digest --seed 42 --round 240 --digest 20：main 与 B4 两棵树 SHA-256 都是
C928C3F19AFE3BA9D36A70DF8E340E3849271574663920D544AE62AFF70B06A9（逐字相同）
逐行逐字段比（脚本口径）：除 events/top_events 外**处处相等**，且这一局里
事件计数**一条都没变**（42 号种子的 240 回合没出现过「整发被点防吃光」）
```

**验收**：`cargo nextest run -P full` **233 绿 / 0 红 / 34 skipped**（main 时 230，新增 3 条 B4 单测）；
`SCHEMA_VERSION` 19 → 20（这一档**真的动了 `State`**：`State::events` 是持久字段；旧档的
`Attack` 靠 `#[serde(default)]` 补空 `shots`，语义 = 「没记」而不是「打了一发没有任何分解」）；
`migrate` 的合并档扩到 `13..=19`；事件表 `data` 列的中文说明补了 `attack.shots` 的全套字段与
两个坑（`magnitude = 0` 的行是真的；`Σshots.damage` 与本表 `magnitude` 只差两位小数舍入）；
kit 新增 **`q.salvos()`**（把 `data.shots` 摊平，含算好的 `score` 列）。

**顺手回填的一条欠账**：`feature/site-supply`（`ad93ad2`/`af97de6`，已并入 main）**改了行为却没
记新基线**——`notes.md` 的「快速参考」里当下仍是 `81A197…1811`，实测**当前 main 已是
`C928C3F1…06A9`**。B4 这一条正好落在它之后，所以两边一起写进去了（见 `notes.md` 的基线链）。

### 6.6 B5 落地记录：两个面的分工（`feature/b5-inputs`）

**这一批先改的是「面」本身，不是某一族量。** 起因是用户对 `pre` 的两问（原话）：

> 「如果回合开始观测的那部分数据不依赖随机/输入，为啥不放 post？此外数据只有依赖当回合的
> state 才应该在 post」
>
> 「现在只有可能未来与随机/输入有关的东西都放 pre，不一定要求当前的实现有关。其他 confirm」

**先把事实量清楚（用户第一问的答案）**：`pre` 当时是 `view_from_state(state)` = 拿**空 sink**
观测回合开始的世界。实测（seed 7 / round 6 的档，与**上一回合**那一行的 `post` 比）：

```text
pre 的观测      ==  上一回合 post 的观测   →  True
post 的观测     ==  上一回合 post 的观测   →  False（世界已经变了）
pre   : production={}, upkeep=0.0, governance_cost=0.0, investment_spent={}
prev  : production={硅:20,碳:0.75,铁:40}, upkeep=20.6, governance_cost=6.0
```

⇒ `pre` 当时**信息量为零**：观测那一半是上一回合 `post` 的**副本**（同一份 state、同一个
`observe`、空 sink 只按中性值表抹平过程量），过程那一半**按构造恒为中性值**（还有一条守卫
钉着）。**第一问的答案是：那半既不该留在 `pre`，也不必塞进 `post`——该删掉**（要读「回合开始
的世界」读上一行的 `post`）。

**第二问把判据换掉了**：文档里「`post` 是 `state` 的函数」这句**在 B1–B4 之后已不成立**
（`labor` 取人口增长前的人口、`is_hub` 取决于回合中段的迁都、`capacity_ledger` 是挂单那一步
算的——都**不可**由回合末 state 重算）。于是真正的划分是**三类**，而 `pre`/`post` 是按**时间**
切的，两套切法错位才是这个面一直别扭的根因：

| 类 | 是什么 | 能不能事后重算 | 归属（B5 之后） |
| --- | --- | --- | --- |
| ① state 派生的观测 | 实力占比/霸权/战争/人口/舰队价值/市场价…… | **能**（`view_from_state`） | `post` |
| ② 回合过程事实 | 产出/维护/治理/成交/运输/逐发/判定…… | **不能** | `post` |
| ③ 回合的**输入** | C7 顺序、C13 噪声、`derived_roll` 家族、判定时看到的候选池 | **不能** | **`pre`**（B5 新装） |

**裁决与实现**（B5a）：

1. **`pre` 的类型换成 `RoundInputs`**（新文件 `src/model/inputs.rs`）——不再是 `RoundView`：
   它现在装 `order`（C7）、`relation_noise`（C13）、`rolls`（`derived_roll` 家族，通用记录
   类型已定：`{purpose, faction, subject, value, threshold, pool_total, picked}`，闸门与加权
   抽签两种用法都能表达）。
2. **砍掉观测副本**：投影/CLI 的循环不再 `pre = view_from_state(...)`，改为把
   `advance_round` 交出来的输入面接住。`view_from_state` 保留，但用途降级为「随时重算一份
   观测」（round 0、`--derived` 无档时）。
3. **产出方式 = Q2 的 (b)**：`RoundSink` 多一格 `inputs`，回合末整份交给调用方。核心函数
   **保留 `advance` 的原签名**（98 个调用点里 92 个在测试里——不为这个改 92 处），另加
   `advance_round(state, config, rng, &mut RoundInputs) -> RoundView`；`advance` 就是它的薄壳。
4. **两个读面都能读到**：`--derived` 的 `pre`（档里存的那一面）+ `--index` 的新派生表
   **`idx/round_inputs.jsonl`**（一回合一行，按 `round` 读）。⚠ **不内联进 `main.jsonl`**
   （C7 的顺序是整份舰名列表，几十个名字，只对「谁先手」有用——与 B4「事件只内联 id」同一笔账）。
5. **`SCHEMA_VERSION` 19 → 20 → 21**：`RoundState.pre` 的类型变了（档的形状变了），旧档的
   `Attack.shots`/`pre` 靠 `#[serde(default)]` 各自补空。

**实测到的两处「回合中段」真相**（写进文档，免得下一个人当成漏记）：

* **C7 的顺序名单不等于回合末的舰集**：它是**洗牌那一刻**的 `state.ships`——含这一回合稍后
  被打沉/除名的舰（死亡清扫在 `step_military` 收尾才做），不含稍后才下水的舰。第一次写用例
  时按「等于回合末舰集」判，第 2 回合就红了（21 vs 16），才发现这条。
* **记录不消费随机流**：C7/C13 都是「读一次用一次」，把值抄下来不影响后续骰子 ⇒
  **digest 逐字不变**（`C928C3F1…06A9`，与合并后的 `main` 相同）。

**还剩什么（B5b/B5c，已定形状）**：

* **B5b · `derived_roll` 家族（约 18 处、8 个文件）**：接入形状是**「骰子由调用方掷、传进去」**
  ——因为这些函数是**纯函数**（`should_be_role` / `route_for` / 闸门判定），而且**同一枚骰子
  可能被问两次**（`should_be_role` 既被「挂单估运力」问、又被「定编拍板」问）⇒ **只在拍板处
  记一条**（记的是「谁做了什么决定」，不是「谁算过」）。
  * ✅ **B5b-1 已落地（9 处：定编 + 合同撮合）**。实践把形状钉死成三条：
    1. 纯函数多返回一个 **`Option<机会值>`**（`role_with_roll` / `observe_with_roll`）：
       `Some(p)` ⇔ 骰子**真被用到**（早退档返回 `None`）⇒ 记账那边**不必复制一遍早退逻辑**
       ——**判据只有一处**，这是这一批最值钱的一条纪律；
    2. 拍板路传 `recorder: Some(&mut inputs)`、估算路传 `None`：**一条判据、两种调用**；
    3. **没掷就没记**（整期无产出的合同不续约 ⇒ 不掷骰 ⇒ 输入面里没有它）——这不是漏记。
    用途清单：`role` / `observe_role` / `route` / `gate` / `accept` / `pick` / `assign` /
    `quit` / `review` / `renew`；实测 seed 7 / 30 回合共 **1106 条**（`gate` 643 条最多）。
    > 样例（直接回答玩家的问题）：
    > `{"purpose":"gate","faction":"俄罗斯","subject":"契约0","value":0.4561,`
    > `"threshold":0.5167,"picked":"heard"}` ／
    > `{"purpose":"gate","faction":"无国界科学组织","value":0.8054,"picked":"unheard"}`
    > ⇒「没人接我的单」原来是**根本没听说**，而不是「听说了不接」。
    ⚠ 用例踩坑：`fresh_world` 把角色轴钉成「全员战舰」⇒ 定编两族骰子**根本不掷**，
    「防空转」的断言必须换**没钉**的世界（`default_state`），否则是在断言一个假象。
  * ✅ **B5b-2 也已落地**：蓝图 `blueprint_intent` / `blueprint_retune` / `blueprint_theme`、
    船坞 `retool`（`war-retool` / `hauler-retool` 两处，`subject` 区分）、风格轴
    `style_chance`、导航 `nav`（MOND 偏航，**幅度骰**：两个判据字段都空，`picked` = 落点）。
    实测 seed 7 / 30 回合 **3944 条 ≈ 127 条/回合 ≈ 21.6 KB/回合**（`main.jsonl` 26.2 KB/回合
    ⇒ 输入面已与整份视图**同量级**），大户是 `style_chance`（逐舰逐轴，1371 条 / 220 KB）与
    合同 `gate`（逐势力逐单，643 条 / 101 KB）。
    > ⚠ **量本身是个发现**（用户说「不管体积」照做，但值得记一笔）：输入面从 B5b-1 的
    > 37 条/回合涨到 **127 条/回合**，就是「逐舰逐轴 / 逐势力逐单」这类**笛卡尔积**型记录撑大的。
    > 若哪天真要压：`style_chance` 的 `skip`（1371 条里绝大多数是「这回合没动」）可以折成
    > 「每舰每轴一行、只记动没动」，或把 `subject` 里的冗余前缀（`长城:doctrine:temper`）
    > 拆成两列。**现在不做**——记全的收益（能回答「为什么它没动」）大于这几 KB。
  * ✅ **B5b-3 / B5c 也已落地**：`observe_body`（观测选靶，加权抽签；读面显示「这艘观测舰去
    哪儿」走的是**不记账**的 `target_body`，只有真的派船时记）；以及 **B5c = 候选池**：
    `Roll` 长出 `pool: [{name, weight}]`，在六个真抽签处填上（`route` 各条腿的货、`pick`
    各家的信誉权重、`observe_body` 各天体的期望在场收益、`blueprint_theme` 各主题权重、
    定编 `role`/`observe_role` 同侧每艘候选舰的票）——它答的是「**为什么是它而不是别人**」，
    而不只是「池子多大」。守卫钉住三条恒等式：池非空、`Σ weight == pool_total`、
    `picked ∈ pool`。API 形状：`record_gate/record_draw` 返回 `&mut Roll`，池子在下一行
    `… .pool = …` 挂上（省掉 7 个参数的签名）。
  * ⏳ **还剩（小）**：风格 `step`（步长那第二枚骰子）**有意不记**——同一个决定的第二个数，
    读面已从轴的新值看出结果，记两次会破「一条抽签 = 一个决定」的口径。* **B5c · 判定时看到的输入**（用户 confirm 要收）：`build_fire_plan` 的候选池、`route` 的
  积压占比、合同的 `eligibility`……它们是**回合中段的 state 快照** ⇒ 属 ③，同样只在拍板处记。
  B5b-1 已经把**判据本身**记进去了（闸门的 `threshold`、抽签的 `pool_total` 就是它们）；
  剩下的「候选池全表要不要逐项收」等 B5b-2 之后再定——那一项会让记录从 ~37 条/回合涨到
  上百条/回合，值得单独量一次体积再决定。

## 7. 待裁决（剩下的设计点）

* **Q1 · 「每次结算 / 舰对」粒度的量放哪？** ✅ **已裁决：(b) 进事件层**（实现见 §6.5）。
  当时的背景：战斗五条（C1–C5）与索敌计划是**逐舰逐发**的，塞不进「每势力一行 / 每城一行」的
  形状。三条路：
  （a）扩 `RoundDecisions`——它已经是「这一回合选了什么、为什么」的家，且 `decisions` 已在
  `main.jsonl` 里**整份内联**（代价：体积）；
  （b）**事件层**（进 `state.events` / `Attack.data`）——可检索、可进稀疏历史，且轨迹里只付
  一个事件 id（约 5.5–7 B），正文落在 `idx/events.jsonl`；
  （c）**不捕获明细**，只把聚合折进 `FactionRow`（如「本回合被规避掉多少伤害」）。
  **裁决 = (b)**；理由与代价（`--derived` 看不到 events、`State` 变胖、`relations` 的交火判据
  必须一起加闸）都记在 §6.5。
* **Q2 · `pre` 面怎么产？** ✅ **已裁决并落地**（见 §6.6）。当时问的是两种形状：
  （a）`advance` 返回 `(pre, post)`；（b）保持单返回值，把 `RoundSink` 扩成也收「判定流水」，
  回合末折两档。**取 (b)**，并且进一步**没有改 `advance` 的签名**（92 个测试调用点不动），
  另加 `advance_round(state, config, rng, &mut RoundInputs) -> RoundView`。
  命名那个问题也定了：**保名 `pre`/`post`**（改动面小、已被大量用例与笔记引用），
  但把两条定义**逐字写进 `RoundState` 的文档**，并删掉那句错话（「`post` 是 `state` 的函数」）。
* **Q3 · 体积**。`main.jsonl` 已经内联整份 `view`（含 `decisions`）。B4 若把逐发索敌计划也算进
  `view`，一局长局的 jsonl 会明显变大。备选：只在 `--derived` 里给、或单独成表 + `--every` 降采样
  （[`coarse-trajectory-views.md`](coarse-trajectory-views.md) 已有这套机制）。
  ⚠ **B1 已经把这个体积问题量出来了**（§6.1）：`main.jsonl` 18127 B/行（+29%），其中 `capital`
  一个对象就占 1350 B/行、`loyalty_target` 里约 40% 是同势力重复的全国项。用户由此提出了
  **「稠密读面 / 自动稀疏存储」**那一层——见 [`dense-face-sparse-store.md`](dense-face-sparse-store.md)
  （若那套落地，B1 这两处不必在「每行自足」与「不重复存」之间二选一）。

## 8. 次级候选（近失，别再重复盘一遍）

* A 组：`production.rs:141 effective`（矿藏封顶）、`construction.rs:155 planning_remaining`
  （地皮余额）、`production.rs:109 building_health`（受损建筑减产）、`construction.rs:301
  launch_blueprint`（同城同舰级下谁的设计图先下水）。**`sim/cities.rs` 无候选。**
* B 组（已排除）：`freight_lean` / `freighter_quota` / `haul_gap` / 合同 `ratio` 已在投影的
  `factions` / `contracts` 表；手续费 / 承运人身份 / 贸易额度余量已被 `net_import` /
  `carrier_income` 折平。
* C 组：`military.rs:296/434 def_mult`、撤退判定第三关 `dist(pos,cap)>retreat_min_dist`
  （`tactics.rs:439`）、玩家舰 `auto_combat` 的火力/轰炸判定（`tactics.rs:242` —— 代码里**自己写着
  「这是有意留下的一格空白」**）、`ship_combat.rs:248 component_effectiveness`、`ships.rs:117
  ship_power`、`power.rs:192 dom`/`scale`、`military.rs:28 focus_of`（联盟集火）、
  轰炸按 `deployed` 分摊的建筑级伤害（`military.rs:439-441`）。

## 9. 复核方式（行号不是印象值）

三条盘点各读一遍工作树，最后逐条回归 `main` = `7e11d32`：

```bash
# A 组（经济/治理）
grep -n 'target_eff\|ideo_penalty\|total_admin\|ent_total\|overload' src/sim/governance.rs
grep -n 'is_hub\|housing_capacity\|labor\|short\|frac' src/sim/production.rs
grep -n 'inv_spent\|con_spent\|increment\|class_rate' src/sim/construction.rs
grep -n 'cur_cost\|best_cost\|old_share\|loyalty_cost' src/sim/capital.rs
# ⚠ 这条当时**只 grep 了 src/**，于是误判成「零调用者」——真正的调用者在 tests/ 里
# （`tests/horizon_long.rs:710` 的探针）。教训：判「死代码」要 grep 仓库根，别只 grep src/。
grep -rn 'faction_ideology_debuffs' --include='*.rs' .
```

行为中性（B1–B4 的验收门，纯追加类改动）：

```bash
cargo run --quiet -p planet_x -- --seed 42 --round 240 --digest 20 | grep '^{' > /tmp/px.txt
sha256sum /tmp/px.txt   # 只取 ^{ 行、\n 连接、UTF-8 无 BOM
# 基线见 notes.md「快速参考」：657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665
```
