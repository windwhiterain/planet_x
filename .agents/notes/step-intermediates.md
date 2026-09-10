# Step 中间量清单：36 条「算完就扔」的量

> 状态 `[ ]` **未实现**（本篇只是盘点，一行代码都还没写）
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

| `文件:行号` | 量 | 粒度 | 骰子 | 它能回答什么问题 |
| --- | --- | --- | --- | --- |
| `sim/governance.rs:215` | `target_eff`（分项 `:213 target_base`、`:214 ent_bonus`、`:207 cap_bonus`） | 每城 | 无 | 这座城本回合的忠诚**目标值**及各分项——距离扣了多少、娱乐预算实换算成多少加成、首都人口占比 buff 多少。`view.factions[].governance_cost/coverage` 只给势力 total，「为什么这座城忠诚在掉」目前**没有解释面** |
| `sim/governance.rs:209` | `ideo_penalty`（`ideology_loyalty_debuff`：`viol_mil/sci/elite/col`） | 每势力 | 无 | 「优势端思潮 vs 行为不符」扣的**全国**忠诚惩罚——为何全国忠诚一起掉（军国却不打仗、科学却不探 MOND）。现成 `pub fn faction_ideology_debuffs`（`:129`）**全仓库零调用者** |
| `sim/governance.rs:173-178` | `total_admin` vs `ent_total`（行政 vs 娱乐拆分） | 每势力 | 无 | 「钱没花在我想的地方」：娱乐预算拉满却被行政（距离 × 人口超载）吃掉。现在只捕获了合计 |
| `sim/governance.rs:171-172` | `overload` / `scale = 1.0 + overload` | 每势力 | 无 | 人口超管理容量后**放大所有远距离城**的治理费与忠诚惩罚——「为什么治理费比上回合暴涨」 |
| `sim/production.rs:133` | `labor`（= `labor_ratio`；`sim/construction.rs:95` 用的是**同一把尺**） | 每城 | 无 | 人口 / 建筑用工之比，直接乘在采矿产出上——「为什么这座城产量低」= 人手不足（人口→劳力的传导点） |
| `sim/production.rs:91` | `is_hub` | 每城 | 无 | 产出**直进势力池**还是**先落产地货栈等船运**——「我挖出来的矿为什么用不了」。`view.cities[].production` 明确只记开采量、不分入库路径 |
| `sim/production.rs:121` | `housing_capacity`（+ `:103 housing_area`） | 每城 | 无 | 人口增长的**住房天花板**——「为什么人口不涨了、产出提不上去」= 住宅面积 × 生态容量封顶 |
| `sim/production.rs:214,216` | `short` / `frac`（`:217` 的 `.max(0.2)`；每舰 `hull_max*frac` @`:223`） | 每势力（落到每舰 hull） | 无 | 付不起维护费时舰队**按比例生锈**——「为什么我的船在掉血」。只有锈到 0 才留 `DeathCause::UpkeepShortfall` 事件，**掉血本身零记录** |
| `sim/construction.rs:20-21` | `inv_spent` / `con_spent`（对比 `investment`/`construction` 限额；写入点 `:218`、`:308`） | 每势力（按资源） | 无（该 step 的 rng 只被 `retool_shipyards`/`design_fleets` 消费） | 本回合**实际花掉的**投资/建造预算——「批了 100 铁为何只花 30」。限额是持久 control 叶（可见），已花量是纯局部变量（写完即弃） |
| `sim/construction.rs:303` | `increment`（+ `:267-272 class_rate`/`class_weight`） | 每城（按舰级） | 无 | 该舰级本回合**实得建造进度** vs 产能速率上限——造舰慢是缺钱还是缺产能；哪个舰级在抢同一笔建造预算 |
| `sim/capital.rs:47-48` | `cur_cost` / `best_cost`（+ `:45 best`） | 每势力 | 无 | **迁都判据数字**：新旧首都的「总治理距离成本」各是多少、候选城是谁。事件 `CapitalRelocated` 只带 `reason` 字符串，**不带数字** |
| `sim/capital.rs:60-61` | `old_share` / `loyalty_cost` | 每势力 | 无 | 迁都当回合对**全国每座城**的忠诚扣减及其来源（旧首都人口占比）——「为什么迁都以后忠诚集体掉了一截」 |

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
  「另一个副本」。
* **乙 · 不持久、但可从回合末 state 重算 ⇒ 捕获的收益是「与引擎逐字一致」**（不必额外维护一条公式）：
  A 组 `ideo_penalty` / 行政娱乐拆分 / `overload` / `labor` / `is_hub` / `housing_capacity`；
  B 组购买力序位 / 禁运三档 / 合同四闸门 / 单次导航成功率；C 组思潮 target / `war_scar_floor` / `aff` /
  `deterrence` / `kiting_dest` / 集体安全级联。
  照 [`unified-metrics.md`](unified-metrics.md) 的老规矩（**总结不该被独立重算一遍**），这批也归引擎。
* **丙 · 吃骰子 ⇒ 读面看不到「掷了什么」**：C7（主 `Prng` 洗牌）、C13（主 `Prng` 噪声），
  加上 B 组 6 条 `derived_roll` 判定（`role` / `route` / `gate` / `accept` / `pick` / 派工退约）。
  **这一批必须进 `pre` 面**（B5）。

## 6. 批次（建议的开工顺序，每批独立可合并）

| 批 | 内容 | 为什么这个顺序 | 是否要动 `advance` 返回值 |
| --- | --- | --- | --- |
| **B1** 治理/忠诚 | A 组 `target_eff` 分项、`ideo_penalty`、行政 vs 娱乐拆分、`overload`、迁都判据（`cur_cost`/`best_cost` + `old_share`/`loyalty_cost`） | `unified-metrics.md` 候选第一条；「帝国为何要崩」的预警面；全是确定性、粒度天然对齐「每城一行 / 每势力一行」 | 否 |
| **B2** 钱去哪了 | A 组 `inv_spent`/`con_spent`、`increment`/`class_rate`、生锈 `frac`、`labor`、`housing_capacity`、`is_hub` | 回答「批了为什么没花」；`FactionRow`/`CityRow` 各加几列即可 | 否 |
| **B3** 市场与运输 | B 组 12 条里的**确定性 6 条**（`p_eff` 分解、丢货、购买力序位、禁运三档、`HaulStep`、`capacity_ledger`） | `HaulStep` 是「货为什么没运回来」的唯一入口（连事件都没有）；`capacity_ledger` 补上「挂单数量从哪来」 | 否 |
| **B4** 战斗 | C 组 `hit`、`armor_soak`、`pd`、`deterrence` + 索敌计划（`build_fire_plan` / `doctrine_weight`） | 玩家最想要的一批（「为什么我打不中」），**但粒度最麻烦**——见 §7 Q1 | 否 |
| **B5** `pre` 面 | C7 洗牌顺序、C13 关系噪声、B 组 6 条 `derived_roll` 判定（含合同三闸门、派单、定编） | 唯一一批**必须**改 `advance`：让它同时产出「AI 看到/掷出了什么」 | **是** |

每批的验收门（缺一不可）：

1. `--seed 42 --round 240 --digest 20` 的 SHA-256 **逐字不变**（取行口径见 §9）；
2. `cargo nextest run -P full` 全绿（快档 ~2 s、中档 ~8 s、全档 ~27 s
   ——`[profile.test] opt-level=2` 之后，见 [`test-wall-clock.md`](test-wall-clock.md) §0）；
3. 读面契约测试（`tests/projection_derived.rs`）加一条：新列在 `pre`/`post` 两档都存在且类型一致；
4. Python kit（`play/planet_xq`）与 web 信息树能**泛化**读出（这两处本来就是 generic 渲染，通常零改动）。

## 7. 待裁决（三个设计点，动 B4/B5 之前必须先定）

* **Q1 · 「每次结算 / 舰对」粒度的量放哪？** 战斗五条（C1–C5）与索敌计划是**逐舰逐发**的，
  塞不进「每势力一行 / 每城一行」的形状。三条路：
  （a）扩 `RoundDecisions`——它已经是「这一回合选了什么、为什么」的家，且 `decisions` 已在
  `main.jsonl` 里**整份内联**（代价：体积）；
  （b）**新的事件层**（`ShipFired` / `TargetChosen` 之类进 `state.events`）——可检索、可进稀疏历史，
  但事件表已经不小；
  （c）**不捕获明细**，只把聚合折进 `FactionRow`（如「本回合被规避掉多少伤害」）。
  倾向：(a) 给 AI 决策、(b) 给「本回合内不可复原」的结算事实（C1–C5）。**要用户裁决。**
* **Q2 · `pre` 面怎么产？** B5 要让 `advance` 同时吐 pre。两种形状：
  （a）`advance` 返回 `(pre_view, post_view)`；
  （b）保持单返回值，但把 `RoundSink` 扩成也收「判定流水」，回合末由 `observe` 一次折出两档。
  顺带一个语义问题：`pre` 现在是「回合开始时的观测」，B5 之后它会变成「回合开始时 AI 看到 + 掷出
  的判定」——**这个名字要不要改**（比如 `pre` 不变、另开一个 `rolls` 段）？
* **Q3 · 体积**。`main.jsonl` 已经内联整份 `view`（含 `decisions`）。B4 若把逐发索敌计划也算进
  `view`，一局长局的 jsonl 会明显变大。备选：只在 `--derived` 里给、或单独成表 + `--every` 降采样
  （[`coarse-trajectory-views.md`](coarse-trajectory-views.md) 已有这套机制）。

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
# 「现成观测函数零调用者」这条是这么查的（只应命中定义那一行）
grep -rn 'faction_ideology_debuffs' src/
```

行为中性（B1–B4 的验收门，纯追加类改动）：

```bash
cargo run --quiet -p planet_x -- --seed 42 --round 240 --digest 20 | grep '^{' > /tmp/px.txt
sha256sum /tmp/px.txt   # 只取 ^{ 行、\n 连接、UTF-8 无 BOM
# 基线见 notes.md「快速参考」：657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665
```
