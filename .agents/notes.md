# 《行星X》笔记索引

> **🡒 当前已裁决、下一步做什么**（2026-10，用户已确认；实现顺序见
> [`control-live-layers.md`](notes/control-live-layers.md) §9）：
> ① ~~**web 三条**~~ → `[x]` **已完成**（`feature/web-control-panel-ux`，提交 `f673bac`；实现记录、
> 实机数据见该篇 §10）→ ② ~~**两轴叶**~~ ＋ ②b ~~**删叶（方案 A）**~~ → `[x]` **都已完成**
> （`feature/leaf-existence`：单轴新建两轴叶被引擎拒（`partial_doctrine_leaf`）、`remove: true`
> 删叶 + `NOTE_APPLY_REMOVED` 回执、kit 的 `remove_*`、web 行上的「恢复出厂值」；见该篇 §11）
> → ②c ~~**角色轴（第三条风格轴）补齐**~~ → `[x]` **已完成**（`feature/role-axis-parity`：运输
> `feature/freight-collection` 合进 `main` 时加进来的 `ship_freighter` / `default_freighter` 当时
> 没走 ②/A 的规矩——没有 `remove`、web 与 kit 都不认识它；现在三端齐了，并查实这条轴是**唯一
> AI 会写的风格叶**（删叶 = 交回自动定编，不是冻结）；见该篇 **§12**）
> → ③ ~~**舰船设计图**~~ → `[x]` **已完成并合并**（`feature/ship-blueprint`，实现提交 `fe534ff`，
> 合并提交 **`89741b8`**：`SCHEMA_VERSION` **9 → 10**，
> 引擎 + `planet_x_ctl` + web 三端齐活，同 seed `--digest` **逐字不变**；实现记录、验收数据、
> 未做项见 [`ship-blueprint.md`](notes/ship-blueprint.md) §6 与规格篇顶部状态行，
> **审查方的独立验收**见同篇 **§7**）
> → ③b ~~**web 的设计图库面板**~~ → `[x]` **已完成**（`feature/web-blueprint-editor`：势力级**设计图库**
> （新建 / 改 / 删 / 三态归属 / `ship_count`）+ 建造区那一行的联动 + **`launch_waiting`（买不起 ⇒ 未下水）
> 第一次看得见** + 引擎守卫的报错回执；引擎只加了**读面**两处（控制读面的 `launch_waiting` 列、
> `/api/command` 回传 `ApplyReport`），`--digest` 与 `--control` 均**逐字不变**。实现记录、载荷原文、
> 实机十二步、两把量具见 [`ship-blueprint.md`](notes/ship-blueprint.md) **§8**）
> 下一步（本轮之后）：`control-live-layers.md` §12.6 的排队项
> 　① ~~**`ship_orders` 读面列出每一艘舰**~~ → `[x]` **已完成**（`feature/read-face-parity`，提交
> `09027a4`：读面**每舰一行**、`behavior` 改成**有效值**（`null` = 链上没人说话）、
> 「`null` 行**不建叶**」的写面规则，web / kit 跟着对齐；见该篇 **§13.1/§13.2**）
> 　② ~~**kit 的 `_approx` 列换成引擎的 `effective`/`order_source`**~~ → `[x]` **已完成**
> （同一分支：`ships()` 优先读引擎的 `order_effective_mode`/`order_effective`/`order_source`
> ——含**设计图层**；只有旧 index 目录才退回本地近似，且列名带 `_approx` + 布尔来源列
> `effective_order_from_engine`；见该篇 **§13.3**）
> 　②b ~~**审查方钉住的洞：kit 那三列的语义被「读面改形状」静默换掉**~~ → `[x]` **已修**
> （提交 `f1be7fb`：`order_leaf`/`order_value`/`order_behavior` 回到**真实的叶**——权威读面 =
> 投影的 `derived.control`；顺带让「叶里的记录值 ≠ 有效值」在 kit 上可读，`demo.py` §[4d] 钉死；
> 见该篇 **§13.6**）
> 　③ ~~**方案 B「逐舰取值规则与文档对齐」**~~ → **已否决**（[`control-value-rule.md`](notes/control-value-rule.md)：
> 模型**没有歧义**、不改，只把取值规则写进文档。理由：AI 写的叶就是 `Auto` 那一档的**值存储**，
> 若让 `Inherit` 不供值，玩家钉的 `Player` 舰队默认会被逐舰 `Auto` 叶压住 ⇒ §8 失效）
> 　④ ~~**风格轴与 `Auto` 设计图的真执行者**~~ → `[x]` **已合并**（`feature/ai-agency`）：
> `autocontrol::style`（风格三轴按战况概率重估）+ `autocontrol::blueprints`（AI 建图/重估/
> 去重/回收）——**§3.2 那条"风格轴没有执行者"的空头承诺还清**；见 `control-live-layers.md` §16–§18。
> 剩下的：`military-combat.md` 的 refit（把新图套到老舰上）＋
> `eras-technology.md` 的时代门控（图库容器已就绪）——**都还等用户裁决**。
> 每步都要过：`cargo nextest run -P full` 全绿（= 全档，含长局）+ 同 seed `--digest` 比较。
> （内循环用 `cargo nextest run`，快档 ~4 s；见 [`notes/test-tiers.md`](notes/test-tiers.md)）
> ⚠ **基线已换代（2026-10）**：`293725C4…DBC4` 是 v10（设计图）时代的基线，已作废——
> 三个合并一起把它换掉了：另一个会话的 `feature/freight-contract`（`05fe04f`，雇佣运力市场，
> `SCHEMA_VERSION` 10 → **13**）＋ 本会话的「读面/写面两侧对齐」`feature/read-face-parity`
> 与 `feature/web-blueprint-editor`，以及 `feature/ai-agency`（**两个 `Auto` 执行者第一次真动手**）。
> 这些**都是有意的行为改变**（运输仍在 WIP，用户明说数值以后再调），不是噪声。
>
> **当前基线（`main` = `52eb2bf`）**：`--seed 42 --round 240 --digest 20`（只取 `^{` 行、`\n`
> 连接、UTF-8 无 BOM）=
> `B6078F7ED778AB13299E9C8B498FDA14BA5E36F1A74BEBD94B163602882BB126`（12 行，连跑两次相同）。
> 对账用（别拿来比合并后的树）：`freight-contract + 读面/web 两条` 时是 `B6F234FB…CB06`；
> `ai-agency` 在自己的旧基线上是 `D693E838…9273`。
> 行为中性的替身（`config/game.ron`）：`autocontrol.style_chance = 0` +
> `autocontrol.blueprint_themes = []` ⇒ 退回旧行为（`control-live-layers.md` §18.1）。
> ⚠ **基线再一次换代**（`feature/ideology-role` 合并 = `88b7c5b`，**有意的行为改变**：
> 思潮决定运输/战斗倾向 + 造舰动机解耦 + AI 估建造时间，见 `ideology-roles.md`）：
> `--seed 42 --round 240 --digest 20`（同样的取行口径、连跑两次相同、12 行）=
> `9A1000019D2198ACA4011E04B76943CB434B935412B3D5775F287799CF816F28`。
> 上面那个 `B6078F7E…` 是**合并前**的 `main`（`52eb2bf`），已作废。
> ⚠ **基线再次换代**（`feature/tech-system-mond`：MOND **掌握度连续化** + 「飞船在异常区」
> 知识渠道 + **特权删掉/1.0 棘轮**，见 `tech-system.md`）：`--seed 42 --round 240 --digest 20`
> （同样取行口径、连跑两次相同、12 行）= `0E3E760DC20F7F4353343449B5051A92B8D5149C32D29632D9480E7C86589A0C`。
> 这一支换过三次：`657F2DC9…6665`（M1 连续化，**同一棵树上逐字节不变**）→ `C726F272…4CBA`（M2 渠道）
> → `0E3E760D…9A0C`（特权删掉 + 棘轮）。
> ⚠ **基线又换代两次**（`feature/tech-system-mond` 继续）：**`A5183C6A…0DC`**（崇拜教初始
> 1.0——有意行为改变；并入 `main` 的读面换代之后合并树仍是这个值，所以那一支对世界演化零影响）
> → **`6E376B8F…573F`**（**观测编队**：角色轴第三态 + 观测定编/选靶/派单，见 `tech-system.md` §11）
> → **`7494A2C8…F446`**（**B1 深空治理**：掌握者的忠诚距离项 ×(1−掌握度)，见 §12）
> → **`81A19749…1811`**（**动机自然竞争**：删掉观测「最多占一半」的上限，三支力量按相对主张
>   水位配给，见 §11.3）。
> 角色轴那次重构（bool → 三值枚举）本身**行为中性**：开/关各跑一遍都是 `A5183C6A…`。
> ⚠ §10.4 那个坑（「恢复继承」撤不掉叶里的值）**已解**：方案 A 落地（引擎 + kit + web 三端，
> 见该篇 §11.1/§11.3）。取值规则本身没动——方案 B 仍留在桌上（§10.4 的表）。

> **这里只是索引。** 每条主题的**具体描述**（怎么设计的、落地到哪一步、实测数据、未做项与理由）
> 都在 `.agents/notes/<slug>.md`：一条主题一个文件，文件名就是它的稳定 ID。
>
> - **找法**：按主题扫下面几张表，或者直接 `grep -rn "<关键词>" .agents/notes/`。
> - **顺序无意义**：分组只是为了让扫读舒服，**组内按文件名排序、组间不分先后，都不代表优先级**。
>   旧的 `ideas.md` 编号（`§1`…`§26`）已废弃，别再写 `§N`；每条笔记头部留着
>   `前身：\`ideas.md\` §N`，纯粹是为了让旧引用还能 `grep` 到（`grep -rn 'ideas.md §25' .agents/notes/`）。
> - **引用别处**：写 `notes/<slug>.md`（在 `notes/` 内部写兄弟文件名即可）；指向别的小节时写
>   `` `agent-play-friction.md` §18.6 `` 这种「文件名 + 小节号」形式。
> - **改文档**：新点子 = 新建 `notes/<slug>.md` + 在下面加一行索引；别往不相干的文件里塞，
>   也别再造编号。落地了就把状态勾成 `[x]`，未做完的把「剩余」列补上——**别让它在对话里蒸发**。
>
> 状态图例：`[x]` 已实现并提交 ｜ `[~]` 部分实现 / 已实现但效果未达预期 ｜ `[ ]` 未实现（候选）
>
> 工作约定见 [`../AGENTS.md`](../AGENTS.md)；想「玩」先读 [`agent-play.md`](agent-play.md)；
> 世界规则看 [`spec.md`](spec.md)。

---

## 治理 · 外交 · 经济

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[~]` | [多极与霸权制衡](notes/balance-of-power.md) | 均势外交已落地：弱者合纵、对霸权遏制、集体安全与经济制裁；后半程最强势力城占降到约 0.5。 | 重建缺口（僵尸 ≤2）；霸权疲劳与峰值 ~0.73 待压 |
| `[~]` | [经济深度](notes/economy-depth.md) | 「挖矿无限」是经济失控的根，拟加矿藏耗竭、距离运费与长期景气循环；均未实现。 | 矿藏耗竭要和行星X 新资源配套，防热寂 |
| `[x]` | [国内市场：资源预算 → 货币预算 → 真实交易](notes/domestic-market.md) | 上层投放资源 + 拨钱，下层按 recipe 在市场买；价格逐回合反馈。已补福利并轨（势力级 `welfare_budget` + 城市福利权重）与逐城货币预算叶（`development_money` / `construction_money`）；默认 `enabled=false` 旧世界不变。 | 城际双向交易、福利专用市场、开启后长局 A/B |
| `[~]` | [贸易与制裁](notes/trade-and-sanctions.md) | 市场已改成**真实交换所**（挂单记名卖家/稀缺定价到 8×/按挂单配给/关系即价格/全面禁运），组件硬门槛让稀缺咬到战斗力，**运费+MOND 承运**让崇拜教垄断柯伊伯带货运，并**删掉凭空重建**（重建改为「派船去复垦」）。 | 平衡待调（整合到 3-4 家）；贸易结构出口端未做 |
| `[~]` | [运输：舰船行为 + 集货 + 雇佣运力市场](notes/freight-collection.md) | 公理「**首都即集散地**」定下对外路线；**运输做成舰船真实行为**：`ShipBehavior::Haul`（常驻路线，腿别由货舱推出）+ 产地货栈（非首都产出必须靠船运回）+ **角色轴**（`Ship.freighter`，像风格一样的三态控制属性，AI 按积压定编、玩家可压住；不影响自动开火/kiting）+ **派单按积压占比抽签**（`derived_roll`，不消费主 Prng 流）。MOND 已按裁决**概率化**：偏移是伪随机范围，深处**没有进不去的目标**，只是要多试几个回合。**雇佣运力市场**（用户裁决）：单子以**雇佣船**的形式，雇主挂单的逻辑与派自己的船同源（自己船不够才挂），单子要求的是**运力（件/回合）**；雇主**周期性考核**受雇方的实测吞吐并反馈信誉（信誉只由考核产生），再按信誉决定续约还是换人；受雇方按自己的运力决定接不接受、要不要提前结束；**船沉没不管，对方派几艘船都无所谓**。 | M0–M2b 与 **M4a–c 已落地**、**M4R（雇佣形态改写）已落地**（本分支 schema 12，**合并设计图分支后 = v13**：两条分支各自从 v9 出发、共用过 v10，见该篇 §7 末）；**A/B 实测：集货把流动经济放大 3.5–4.7 倍**；瓶颈仍是舰队里没有货船（每趟只装 3.4–4.9 件）⇒ Q9。雇佣市场实测（seed 3/7/11 × 400 回合）：搬到位 **917–2011 件**、**实付抽成 15.7–17.3%**（开叫 15%）、考核好评/差评 40/18・29/13・15/34、平均达标率 1.06–1.40（修掉「尺子坏了」之前的 6.10）、关系多数**跑满固定期**（抽手仅 11–22 次） |
| `[x]` | [站点自给：消耗只吃本地库存](notes/site-supply.md) | 用户 follow-up「非首都的投资的资源也需要从母星运输过去（或者使用本地产出）」+ 三条裁决（**完全禁止瞬移 / 本地货栈一份两用 / 进承包商**）。`State::stock_at` 成了**唯一**读法（首都 ⇒ 池子、其余 ⇒ 货栈），建楼 / 造舰进度 / 出厂模块全从那处出；`haul_load` 因此也能从首都池装货 ⇒ **补给腿**（首都 → 站点）成立，与集货腿合成**一张按量抽签的表**，承包市场同样吃两条腿。保留量 = **消耗速率 × 这条线一个往返的回合数**（lead-time 库存，与合同考核同一把尺子），进出口互为镜像 ⇒ 不往返乒乓。实测修掉三个死结（保留量按「整个计划」⇒ 全世界 0 舰 / 船体与模块不在需求信号里⇒裸舰螺旋 / 「一条腿一艘船」⇒涓流腿吃光舰队），并顺手修了既有 bug：**倒戈的城带着旧主图纸指针 = 永久停产**（实测 20 个建造区 18 个悬空）。 | **A/B 单开关未做**（该篇 §6 给了做法）；长局 400+ 未系统测；`agent-play.md` 还没写「怎么派补给船」 |
| `[~]` | [思潮 → 角色：运输还是战斗](notes/ideology-roles.md) | 角色轴（`ship_freighter`）的**来源**从硬定编改成**思潮驱动**：`尚武度 = 军国 − 殖民`（两轴同权反号、写死）⇒ `倾向 = 2σ(−1.5×尚武度)`（**中庸 = 1.0 = 旧硬定编**，行为中性）⇒ 目标头数 = 有积压的货栈数 × 倾向；派人是**按缺口抽签**（与 `route_for` 同一条纪律：概率分布 = 想要的比例 ⇒ **期望入伙数 = 缺口**、目标处概率恰为 0 ⇒ 不抖），票按运力效率（软排序，最好的船 ≈ 20 倍票）。实测：平均头数贴着配额（±0.06）、配额处头数 ±1 呼吸而人员**一直在换**（0.52 条/回合）；**第一版「每艘舰各自掷一次身份」实测会在 0 与 12 之间两极震荡**。观察面加 `freight_lean`/`freighter_quota`/`freighter_count` 三列（分开「思潮不让跑」与「没人可派」）。 | **角色轴 + 造舰动机解耦都已落地**（本分支）：角色 = 思潮配额按缺口抽签、**轮换**（头数稳、人员流动）；造战斗舰 = **敌对国与自己的实力差距**（连续量，取代「是否处于战争」布尔）；造货船 = **搬不动的比例**（雇得到人就不造）→ 腾**一个**船坞，两条动机各占一个船坞；**AI 会估建造时间**（`build_rounds`，与真实建造同源）——第一版货船舰级只看运力 ⇒ 全世界船坞改成下不了水的航母、600 回合只拆平 4 次，改成「运力÷建造时间」后同一局 37 次。长局发现思潮自己会饱和到殖民端、动机均值偏满（0.79–0.89）。⚠ **配额口径已改**：`needed_haulers = Σ_腿 min(1, 货量 ÷ 一个货舱)`（见 `site-supply.md` §3.3），不再是「一处积压一条腿 = 一艘船」 |
| `[~]` | [治理与忠诚度](notes/governance-loyalty.md) | 治理开销与忠诚按「距首都 × 人口超载」计费，付不起就离心；已实现城市倒戈换主。 | 「倒戈目标要治理得住」的过滤；忠诚影响生产未做 |
| `[ ]` | [四系统交叉审查：国内/预算/运输/国际](notes/domestic-budget-transport-market-review.md) | 审查发现 3 个 P0（`haul_load` 忽略出口保留量、逐城预算被全局 spent 吞掉、市场把货栈产出算成消费）+ 5 个 P1（`iterations=0`、MOND 吞吐、撮合超卖、价格用总库存、welfare 语义）+ P2 清单；附文件:行号、修法与守卫。 | 另一个 session 按 P0→P1→P2 修，补守卫并重跑全门；P0-3/P1-2/P1-4 会改默认行为，需重标 digest。 |

## 军事 · 科技 · 天文

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [战斗行为风格](notes/combat-behavior-doctrine.md) | per-舰 `doctrine`、逐武器索敌与统一权重已落地，思潮实验也接上。 | doctrine 扩到经济/造舰；政治系统 M1–M3 |
| `[x]` | [科技体系：以 MOND 为干线](notes/tech-system.md)（设计长文） | **道 + 术**：一根连续干线 `mond_control ∈ [0,1]`（**已取代**二元的 `masters` 名单；开局表里**只有行星X崇拜教 = 1.0**——它是唯一天生就懂的势力，1.0 是棘轮 ⇒ 永不下降、永不被人超过）＋ 挂在它上、**按概率习得**的应用项（未做）。掌握度**只连续地改「一次导航尝试的胜算」**：`p = min(1,(eps/(depth×drift×(1−c)))^(1/shape))`、前沿 `r* = 28 + 2/(1−c)`（0→30.0／0.35→31.1／0.7→34.7／0.9→48.0 AU）⇒ 柯伊伯带从「试十几次」变「当月到位」，**没有一步是「解锁了才准去」**。**已落地**：M1 连续化（零回归）+ M2「**飞船在异常区**」这一条知识渠道（在场强度 `Σ(1+深度×0.25)` → 目标 `1−e^(−p/2)` → 按 0.03 松弛；**学满 = 持续 48 个够格的回合**，够格 = 在场强度 ≥ 12 ⇒ 正好到 1.0 ⇒ **棘轮**）+ **§11 观测编队**（角色轴第三态：`ShipRole {War, Freight, Observe}`，`autocontrol::knowledge` 定编/选靶/派单，优先级 **观测 > 运输 > 战斗**）+ 观察面六列 + 市场最后两处布尔改连续。 | **动机已补上，实测有效**：seed 42 的**中国 r120 还是 0.00 ⇒ r240 已经 1.00**（在场率 59%）；seed 7 没人学到的**原因已查明**——那个世界到 r240 **9 家只剩 1 家**（0 城 0 舰），根本没有舰队可派（§9.1 那个停摆项的更强版本，留给平衡轮）。**好处按裁决只做了 B1 深空治理**（§12）：掌握者的**忠诚距离项 ×(1−掌握度)** ⇒ 深处不再离心倒戈；世界级 A/B：r240 有城的势力 **1 家 → 3 家**（方向对：更难被吞并），但**舰队崩解**这个停摆项仍在且更早（三家 0 舰）。**动机改成自然竞争之后（用户裁决「不许加阈值」）**：观测那条「最多占一半」的上限删掉，三个动机（威胁／积压／知识缺口 × 各自思潮倾向）按相对主张**水位配给**整支舰队（§11.3）。实测从「一家独占」变成**扩散**：seed 42 有 **4/9** 家越过 0.35（无国界 0.87、欧盟 0.86、美国 0.69），但**没人拿到棘轮**（除了天生的崇拜教）⇒ MOND 成了「要一直付费维持的流量」。仍待办：其余六条红利（§10 的 B2…B7）、术/`requires`（裁决 6/7）、其余四条知识渠道（裁决 3）、承运费并列时「先到者赢」的 tie-break（§11.2）、`mastery_presence = 12` 要不要调（§11.3.3，我留着等你拍）。 |
| `[ ]` | [时代与科技演进](notes/eras-technology.md) | 用解锁式舰级、材料升级与舰种分支，给上千回合铺时代节奏；三条都还只在纸面（落点见 `tech-system.md` §4「术」）。 | 舰级解锁、结构演进、设计图分支全未开工 |
| `[~]` | [军事与战斗](notes/military-combat.md) | 拟真战斗与舰船定制（组件/护盾/点防/命中折减）已落地，AI 拟人化那批也做完。 | 舰船退役换装、换模块/再装配；长局可玩性 |
| `[x]` | [舰船设计图](notes/ship-blueprint.md)（设计长文） | 非控制属性（面板/选装/造价）放在**建造单位**上作为出厂快照的设计图；也是「还不存在的实体的规则」的家（含按舰级默认）。 | 全部（`feature/ship-blueprint`）；文内 §3 四条语义已裁决（快照 / 三态 / `choose_loadout` 降级 / refit 出本轮）；实现记录见该篇 §6；**web 的设计图库面板（新建/改/删 + `launch_waiting` 可见）见该篇 §8**（`feature/web-blueprint-editor`） |
| `[x]` | [舰船设计图：实现规格](notes/ship-blueprint-spec.md)（**十条已裁决**） | 现状核实（带 `文件:行号`）、数据结构、config/叶片/投影形状、迁移、测试计划、改动地图；更正旧 note 两处事实（出厂风格 config 从未填过、造舰只剩两条路）。**§8.0 = 十条裁决**（Q1 图压舰队默认但意图轴默认沉默 / Q2 活层 + `order_source` / Q3 `ship_type` 仍是唯一真相 / Q4 买不起就不下水 / Q10 悬空指针停产报错…）。 | 已按「附 A 改动地图」实现完毕（`SCHEMA_VERSION` **9 → 10**，`spawned_round` 一并落地）；顶部状态行写着实际提交、验证数据与**偏离项** |
| `[~]` | [MOND 引力异常](notes/mond-anomaly.md) | 异常区导航偏移已实现，且**「崇拜教免疫」已升级为连续的掌握度** `mond_control`（1.0 = 今天的 cult、0 = 凡人、中间是前沿外移；见 `tech-system.md` M1）；战斗光环、矿产红利与科技扩散未做。 | 异常区战斗光环；矿藏加成；MOND 扩散（三条的落点都在 `tech-system.md` §4） |
| `[ ]` | [行星X 回归](notes/planet-x-return.md) | 把行星X 做成第 19 号长周期天体 + 全球回归效应；现在只是第 60 回合的纯散文节拍。 | 天体、回归效应、配置、harness 断言全未开工 |
| `[x]` | [舰级点防修正](notes/ship-class-pd-mult.md) | 五级舰按 spec 重新定性并新增 `pd_mult`，联动模型/配置/选装/meta，测试与长局全过。 | — |

## 剧情 · 历史

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[ ]` | [政治系统设计](notes/political-system.md)（设计长文） | 议题—立场—关切度 + 三因素关系模型，给权力关系提供「目的」；整篇都还没实现。 | 全部；MOND 是第一个议题实例 |
| `[x]` | [稀疏历史设计原文](notes/sparse-history-design.md)（设计长文） | 三层历史的完整设计 + Stage A–E 落地记录 + 复现步骤。 | 文内 §5「未做 / 后续」 |
| `[x]` | [稀疏事件历史](notes/sparse-history.md) | 三层事件历史落地：补因果字段、状态漏斗化、清空里程碑层、断掉拆平/复垦极限环。 | 战争进入仍无迟滞；窗口层的下一读者待定 |
| `[~]` | [剧情与编年史](notes/story-chronicle.md) | 扩展剧情的**机械后果**（拆迁/赠建筑/势力陨落/共同敌人联盟）；目前多只写 `chronicle`。 | `StoryEffect` 扩充、多结局、共同敌人触发 |
| `[x]` | [讲故事模式](notes/storytelling-mode.md) | 已转成导演模式：删掉自研 jq 与查询 REPL，改批次轨迹生成器 + 外部 jq 为必选依赖。 | — |

## Agent 控制面 · 游玩体验

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[~]` | [agent 控制面](notes/agent-control-api.md) | 已把 coalition 格局与城市 loyalty/距离暴露给 agent；summary 级联与目标模板仍缺。 | summary/delta 级联；control 目标模板命令 |
| `[~]` | [agent 游玩摩擦](notes/agent-play-friction.md) | 六类摩擦已补护栏与控制面预览（`--control-schema`/`--control-plan`/`--profile`）。 | 语义指令助手、语义视图命令仍是空白 |
| `[x]` | [agent 游玩打磨](notes/agent-play-polish.md) | 真以 agent 身份玩了一局，修掉「失败看起来像成功」并重写手册，行为中性已验证。 | 语义指令助手、`--control` 瘦身（其余低危） |
| `[~]` | [引擎=数据平面，Python kit=策略平面](notes/engine-data-plane.md) | 引擎产出 tidy 统计表 + 接受同形状 diff：`faction_process`/`city_process`/`control`/`scope`/**`decisions`** 五表 + `--derived` 已落地，消费者（`planet_xq`/`planet_x_ctl`）也接上了；**`--control` 读面不再舍入**；**「AI 掷了什么」已捕获**（逐舰判定 + 船坞改装，行为中性已实测）；**有效指令的链只由引擎算**（kit 的 `*_approx` 降级为旧 index 目录的兜底）。 | 玩家舰的自动战斗判定；逐武器火力分配 |
| `[~]` | [控制属性 = 活层](notes/control-live-layers.md) | 三态归属 + 写值即接管 + **风格活层（doctrine/kiting/role）**全部落地；§4 四条已裁决；⚠ **舰队级「默认指令」2026-10 已删**（见 [`blueprint-stance.md`](notes/blueprint-stance.md)）；**§8 = 控制面板七条裁决**、**§9 = 已确认的动手顺序**。**§10 = web 三条已落地**（`f673bac`）；**§11 = 两轴叶 + 删叶（方案 A）已落地**；**§12 = 角色轴补齐**；**§13 = 读面/写面两侧对齐已落地**（`feature/read-face-parity`：`ship_orders` 读面**每舰一行**、`behavior` 取有效值（`null` = 没人说话）+ 写面「`null` 行不建叶」+ kit 的 `_approx` 换成引擎的 `effective`/`order_source`，`order_*` 回到**真实的叶**）；**§16 = 风格三轴的真执行者**（`autocontrol::style`：按战况概率重估、分布步长、`derived_roll`、三道玩家闸门）、**§17 = `Auto` 设计图的执行者**（`autocontrol::blueprints`：AI 建图/重估/去重/回收，图库 `O(主题×舰级)`）——**§3.2 那条"风格轴没有执行者"的空头承诺已经还清**。 | 两个执行者都还不够平衡（§17.3 的"买不起不下水"扩到 AI 图、玩家图的选装复用、主题权重再标定）；方案 B **已否决**（见 [`control-value-rule.md`](notes/control-value-rule.md)）；文档收尾（把取值规则写全） |
| `[x]` | [设计图带倾向、指令只剩逐舰叶](notes/blueprint-stance.md) | 用户裁决「**指令是即时操作，风格/角色才是长期控制项**」：`Blueprint.order` 与舰队级 `default_ship_order` **两片叶删除**，图改带 `doctrine`/`kiting`/`role`（逐轴独立、活层、插在舰队默认之前）；`SCHEMA_VERSION` 21 → 22；实测证据（那片"默认叶"其实是全舰队接管开关）见笔记 §2。 | 倾向三轴的"是谁供的值"读面（`ship_*_source`）；批量下令动作；AI 是否需要写图上的倾向 |
| `[x]` | [控制属性的取值规则](notes/control-value-rule.md) | **裁决：模型没有歧义，不改。** 归属与取值是同一条链上的同一件事（`Inherit` = 没意见、让位给上面的 `Player`；`Auto` 那一档的**值存在逐舰叶里**⇒「叶存在就供值」）。§1 六行实测表逐行由此推出；§3 结清三条"看起来像歧义"的旧账（恢复继承不还值 = 交互落差，出口是删叶；文档只写了归属那一半；势力默认叶 `Auto` 档没有存储）。 | 把 §1 写进 `src/control.rs` 顶部 + `agent-play.md`（文档收尾）；方案 B/B3 已否决 |
| `[~]` | [Lazy 索引分析层](notes/lazy-index-pandas.md) | 重型字段拆成按 id 的懒表，Python/uv 套件读 schema 后 join 分析；**新增 `derived` 段与 `q.derived(...)/q.control()` 等派生表读法**。 | parquet、更多 lazy 字段、剩余统计函数 |
| `[~]` | [长局控制面缺口](notes/agent-control-long-game.md) | 192 月长局实测：预算只能限速不能封顶、无外交/交战规则/放弃城市叶片、结构性叶片所有权不明、幽灵权重。 | §5 新舰默认归 AI 已被 `control-live-layers.md` 解掉；其余全部（§1 维护费上限、§2 ROE 最关键） |

## WebUI

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [自动选空闲端口](notes/web-auto-port.md) | dev server 从 3000 起向上扫空闲端口，`PORT` 三种语义并回显实际 URL，已实机验证。 | `PLANET_X_WEB_HOST` 放开监听地址（安全决定） |
| `[x]` | [服务生命周期](notes/web-lifecycle.md) | 启动者租约 + 关页即退解决孤儿进程锁 exe；`scripts/web.ps1` 成为唯一入口。 | 无存档；浏览器崩溃不自退；Job Object 兜底 |
| `[x]` | [3D 地图渲染](notes/webui-3d-rendering.md) | 用户报的 4 项 3D 观感问题全修完：光照法线、尺度、材质、遮挡。 | 9 舰同屏标记去重；太阳表面粒面与边缘变暗 |
| `[x]` | [3D VFX 管线](notes/web-vfx-pipeline.md) | 3D 渲染层整体重写为高端 VFX：程序化星空/银河（cubemap 银河 + Points 恒星）、太阳光球+日冕、行星地形/云/大气壳/双向环影、小行星带、HDR 后处理（bloom/god rays/拉丝/鬼影/调色）、五档质量 + 自适应分辨率。另记三个烧时间的坑：GLSL 模板里的反引号、着色器编译失败=物体不渲染、**浏览器默认跑在核显上**。 | WebGPU 迁移（TSL 重写，待裁决）；把 perf/interact 探针提升进 scripts/；云层未独立漂移 |
| `[x]` | [太阳的日珥：大片曲面条带 + 本地空间纹理](notes/sun-prominence.md) | 用户拿 SDO/AIA 304Å 参考图要「很多须须喷发出来」的日珥。走了**三条弯路**（等半径球壳 ⇒ 并入日冕体积分 ⇒ 每像素解析求交）**都出不来须**：掠射时视线几乎与日面平行，积分把三十多根针**平均成雾**（不是参数问题）。细针插片版（几千根）密度与细腻只能二选一，被**第二版取代**：**少数大片曲面条带**（`PROM_MAX=5000`，宽 0.075R~0.34R）+ **片元在世界空间场里把带子切成十几~几十根细长日珥**。⚠ 用户纠正过一处：共享的世界空间噪声管的是**带子的扭曲/朝向**，**带子内部的纹理是本地空间**。另有：格点噪声的**方格纸**（新增 `px/noise/{rot,perlin}.glsl`）、配色从参考图**反解** HDR 锚点、几何 **LOD**（画质档 × 日面屏幕半径，按太阳到相机的距离）。 | 日珥的**弧（loop）**还只是 12% 的小种群；盘面网状纹理可再推一步；插片在**核显**上还没量过 |
| `[x]` | [截图 / 场景测试框架](notes/shot-harness.md) | `scripts/shots/`：零依赖 CDP 截图器（自带 headless Edge，9333）+ 命名场景（固定 URL/相机/**冻结时间轴**/隔离被测对象）+ 像素判据（轮廓参差度/环带/直方图/签名基线）+ `--stats`（量任意图，参考图标定）+ `--solve/--hdr`（ACES 正反算，屏幕色 ⇄ HDR）。
**两条铁律**：服务必须是本 worktree 的（核 `/api/ping` 的 exe）、浏览器必须是自己的（共享浏览器会被别的会话导航走）。 | 还没接进 `check-js.sh`（需活服务 + 真 GPU，是否进门**待裁决**）；`perf/interact` 探针未搬进来 |
| `[x]` | [着色器编译把浏览器卡死](notes/shader-compile-stall.md) | 用户报「用核显打开就卡死」——**归因错了**：真因是 `#define FBM_OCT` 常量上界让 `planet` 片元的两百多个内联 `vnoise` 被完全展开，`fxc` 冷编译 **13 s→31 s→永不结束**（`oct` 越大越糟），GPU 0% / CPU 满载、GPU 进程占死⇒所有标签页一起卡。改成 **uniform 八度数**（不 inline）后各档均可完成且停顿**不再随 `oct` 增长**。**同时记下我上一轮「163fps 全过」是热着色器缓存的假通过。** | 冷编译仍残留 6–14 s（热 1.19 s）：需砍 `fbm` 调用点（②，17 处含 relief 法线差分）或把噪声烘成贴图；`diag.mjs` 提升进 `scripts/` |
| `[x]` | [面向人类的读面：声明式组织点 + 通用兜底](notes/web-human-views.md)（设计 + 实现） | 现状核实「**读面通用但无组织，写面有组织但硬编码**」＋实机量出的缺口（118 KB/帧、`MAX_COLS` 静默丢列、推进 20 回合零反馈）；**已实现**：声明式 `views.json` + 通用求值器 `specview.js` + **铁律 R「只能整理，不能隐藏」**（残差 = 集合差 ⇒ 扩展自动可见，含一次引擎加字段的扩展实验）；**引擎一行未动**（`C928C3F1…06A9` 与 main 逐字相同）。 | 下一步：**D8 Rust 结构体字段按重要性重排**（换 digest 基线）；D5 写面 spec 化；事件行 `headline` 进 web 读面（见该篇 §10.6） |
| `[x]` | [读面：没被声明的字段 = **追加的普通列/普通行**](notes/web-read-append.md)（用户裁决，已落地） | 用户原话「**我不希望有"其余"这样的栏目**」「你就不能直接把没组织的并在后面吗，你把它藏起来我看都看不见」⇒ 残差的**折叠桶整条删掉**（`其余` 列、`▸N` 按钮、「其余字段（N）」、`sv-th-res`/`sv-residual` 三个类名），改成按**引擎字段序**追加的普通 `<th>/<td>` 与普通 `.sv-sheet-row`；列头就是字段名、走与手工列**同一条** `ctx.tip` 查词链；对象值走 `map`（`思潮` 四条轴 / `关系` 八家**全部看得见**，不是 `{4}`），绝不出现 `[object Object]`。**引擎一行未动**（只动 `specview.js`/`style.css`/`app.js` + 两把量具）。 | 新判据 `g4_spec.py` **§8**（四条，口径见该篇 §4）：①读面覆盖 `声明 ∪ 追加 ∪ 不看 == 全部引擎字段`（实测 14 表/207 记录/184 字段 = 79+100+5，**跑真世界 + 用 Node 把 `specview.js` 原样跑起来算** —— 判据不抄第二份口径，量具 `_append_probe.js`）；②静态「没有折叠桶」；③追加列的中文名词覆盖率（42 个全命中；引擎内部 ASCII 槽位名 43 个如实记账）；④**投影对账** `(声明∪追加∪不看) ∩ 投影列 == 投影列 ∩ 实体 schema 字段`（`--index` 的 `schema.json`，7 表/169 列/71 交集）。反向验证 ㉙（剪断追加路径 ⇒ §8a 红）㉚（桶回来 ⇒ §8b 红）㉛（删掉一列 ⇒ 不红，但文本必须指出「这一列现在只能靠追加」）。实机：势力页 27 手工列 + **7 追加列**、卡片 8 条普通行、`Tip.stats()` nouns 308/attached 1101、`__errs == []`。 | **未完/拿不准**：`events` 表 30 个追加列里 11 个 ASCII 槽位名（`GameEvent` 变体字段）**没有 `///` ⇒ hover 弹不出解释**（引擎缺命名，不在本步范围）；事件表按「各行键并集」算列 ⇒ 单行大部分格子是 `·`（要收窄得改**声明**，不是把追加藏回去） |
| `[x]` | [读面第 9 步：**页级**兜底桶也删掉（「未组织」页）](notes/web-read-append.md)（§7，用户裁决「要」，已落地） | 第 8 步删掉了**列级/行级**的「其余」折叠桶；左栏还有一张 `app.js::sidePages()` 现拼的**自动「未组织」页** = 同一个概念的**页级**版本，用户答**「要」**一并去掉。删：`page.id==='leftover'` 分支 + `renderLeftover`/`renderSpecCheck`/`renderWriteCheck` + **只**被它们调的 `shapeOf`/`isNilLike`/`checkRow`/`jsonToggle` + `controls.js::audit()` + `specview.js::claimedPaths()` + 7 个只为它存在的 CSS 类；`views.json` 只改了「全局」页那句**假话** hint（原先说"在未组织页也看得到"）。**信息没丢**：①「哪些字段没被整理」的审计由 §8a（`声明 ∪ 追加 ∪ 不看 == 全部引擎字段`，14 表/207 记录/184 字段）接手；②「看原始数据」开关**本来就是个 no-op**（`jsonToggle` 只 `return btn`，box 从未 append 进 DOM）⇒ 删掉，同一件事右栏「状态」面板在做。**引擎一行未动**。 | 新判据 `g4_spec.py` **§9**（一条含四支：声明里 id/title、`views.json` 全文、`**/*.js` 去注释代码、**且**第 8 步追加接线 5 个标记必须仍在——防"用删掉追加来代替删掉桶"）；防空转数字 7 页/17 份 JS/接线 5-5；非恒真 = `_g4_negative.py` ㉜（页塞回声明）㉝（`.concat` 塞回代码）都要求 §9 自己红，㉚（断 `sv-th-auto`）顺带点亮第 ④ 支，**38 个注入错全咬住**。实机：页签 7 张无「未组织」、势力页 23 声明 + 7 追加列、`__errs == []` | — |
| `[x]` | [写面 spec 化 + 读/控制**穿插**](notes/web-control-spec.md)（**两步都已落地**） | 用户裁决「**控制和读面要穿插在一起**」：把写面（今天 `app.js` 里约 1200 行手写领域代码）也变成**声明**，并让它与读面住在**同一份声明、同一条行序**里——「一个势力的库存（读）」紧挨「它的资源预算（控制）」，不再分两个 tab。要害是**叶种类注册表由引擎发结构事实**（web 与 kit 今天各手抄一份，`role-axis-parity`/`blueprint-stance` 都栽在这个缺口上），前端只发呈现；纪律检查**只用 Python**（新组 `g4_spec.py`；`web/src/views_tests.rs` 搬过去后删）。 | **两步都已交付并合进 `main`**：① `328d4c2`（`feature/web-control-spec`）——引擎发 `leaves`（14 叶 + 1 命令，与 `schemars` 的属性集**双向相等**）、`views.json` v2 四种行（`path`/`leaf`/`owner`/`action` 同住一条行序 ⇒ 库存（读）紧挨预算（控制））、`controls.js`、左栏两模式合并、`g4_spec.py` + 它的量具 `_g4_negative.py`、删掉 488 行 Rust 检查；② `7b9e247`（`feature/control-tree-retire`）——**手写控制树整条删除**（`app.js` −500 行），设计图库进新「设计图」页、建筑权重在城市卡片里修好（以前**根本改不动**）、全局作用域进「全局」页，**kit 的叶种类表改读引擎**（三端同源）。门：Rust 217/217 + web 24/24 + Python 四组 76/76 + 反向验证 19 个注入错全咬住；digest 逐字节不变。**只剩**：用词统一（用户：最后再来统一） |
| `[x]` | [字段顺序 + **序列化中文名**](notes/field-naming.md)（**主体已落地**：批 A→第 5 步） | 用户裁决：**Rust 字段按重要性重排 + `#[serde(rename = "中文名")]`**（名字参考 `spec.md`），让 UI **直接用键名当名词、用 `///` 注释（经 schemars 的 `description`）当悬停弹窗**，不再有任何单独的翻译表；`views.json` 的 `label` 退化成"只在需要覆盖时才写"。实测：`src/model` 80 个结构体 / **627 个字段**（85% 已有 `///`）。 | **已落地**：① 实体字段中文名（11 结构体 83 字段，`45d9cdb`）；② 控制叶/patch 键/读面连接键（`d7490a8`，`SCHEMA_VERSION` 24→25）；③ `control` 表的 `kind` 词表与叶名词统一（`6b91bee`，唯一声明处 = `LEAVES[].field` + `kind_of()` panic 兜底）；④ 悬停弹窗（`--nouns`/`/api/schema` + `tip.js`，`704d008`）与表达式列 `noun` 声明（`fbc3f08`）；⑤ `identity_key` 进引擎、删掉 `_harness._ID_KEY` 等镜像表（`2f6cc7a`）；⑥ **控制行**（`owner`/`action` 行）的 `noun` 声明——它们的**键名不是前端会去查的词**（`app.js::nounTip` 的 field 链里没有 `owner`/`action`）⇒ 不声明就 hover 无反应，而同轮把 `g4_spec.py` §5 的控制行那一支改成**前端等价口径**（`frontend_words`：声明优先 → 叶字段名/裸字段名 → 显示标签），量具 `_g4_negative.py` ㉗㉘（第 7 步）。**纪律**：每个词表只有一处声明（`serde(rename)`/`LEAVES`/`IDENTITY`/`LAZY`），并配对账判据 + 反向注入。⚠ **有意保持英文**的字符串（`derived_roll` 的盐、`SCHEMA_VERSION`、`temper`/`lone_wolf` 的盐用法、`ShipBehavior` 变体名、表名、kit 自有帧列名）见该篇 §8。**未完**：`decisions.kind`/detail、事件载荷、`market`/`salvos` 等派生列（批 B/C）、`config/game.ron` 的键（批 D） |
| `[x]` | [文档注释 → description 管线](notes/doc-pipeline.md) | 悬停弹窗里 46 条文案**以单个 `*` 开头**（源码明明是 `**加粗**`）——根因是 schemars 0.8.22 的 `get_doc` 把「**所有行**都以 `*` 开头」当成 `/** … */` 风格逐行剥一个 `*`（**单行**注释中招，多行幸免）。用户裁决**升级 schemars**（0.8.22 → **1.2.2**；1.x 把 doc→description 整块重写成 `_private/rustdoc.rs::get_title_and_description`，**没有剥 `*` 的逻辑**）。升级代价：方言用 `SchemaSettings::draft07()` 钉死（否则 `$defs`/2020-12 会打断 `g4_spec.py`/`neutral.rs`/`tip.js`）、段内换行归一补回 0.8 语义 ⇒ 升级前后**逐条对账只有那 46 条变**（其余是 `required` 顺序 / `0.0` vs `0` / `const` vs `enum` 这类语义等价的形状差）。新守卫 `g4_spec.py` **§5b 文档对账**：615 条 `description` 与源码 `///` 逐字对账（正向）+ 395 个带注释字段都真的发射了（反向），量具是 `_g4_negative.py` ㉓㉔。 | 三个 `#[serde(into/try_from)]` enum（`DeathCause`/`SpawnVia`/`FoundingHow`）的变体 `oneOf` 不再发射（旧的那份枚举值写的是 Rust 变体名、**本来就是错的**）；要真正发对值得让 `stringly_unit_enum!` 顺手生成 `JsonSchema`，见该篇 §4/§5 |

## Schema / 查询 / 数据面

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[~]` | [超长轨迹降采样](notes/coarse-trajectory-views.md) | 超长轨迹可用 `--every`/`--digest` 采样粗看；极致画像目前只有纯 jq 版。 | 分层缩放 `--zoom`；窗口一句话事件文案 |
| `[x]` | [Lazy 索引分析层](notes/lazy-index-pandas.md) | 重型字段拆成按 id 的懒表，Python/uv 套件读 schema 后 join 分析。 | parquet、更多 lazy 字段、剩余统计函数 |
| `[~]` | [用 Python 统计地编辑控制面 diff](notes/python-control-authoring.md) | `planet_x_ctl` 已建（`play/` 并列 uv 工程）：批量改归属、编制表、统计配方、`verify` 全在 demo 里自断言通过；**风格两片默认叶已接上**（`LEAF_KINDS` 缺口补掉），单轴新建两轴叶当场拒绝。 | `agent-play.md` 一节；`surface()` 换 join 后端并删掉本地 `_approx` 重算 |
| `[x]` | [名字即唯一 key](notes/name-as-unique-key.md) | 实体身份统一用名字作主键，删掉数字 id 与影子结构。 | web 投影收敛；派生字段预计算 |
| `[ ]` | [Schema/查询架构调研](notes/schema-query-architecture.md)（设计长文） | 四套并行投影、隐式 schema、宽容查询、无版本迁移——诊断 + 分层方案。 | 文内 §6「触手可及的首步」 |
| `[ ]` | [Schema 查询重构（候选清单）](notes/schema-query-refactor.md) | 四痛点里的 P0–P3 已落地：`meta_value` 止血、schema 自描述、响亮失败、版本迁移。 | P4 收敛并行投影（拆 `AgentState` 镜像 struct） |
| `[ ]` | [语义视图 API](notes/semantic-view-api.md) | 把裸 jq 降为逃生舱；语义 view 工具只在 jq 侧做了 PoC（压缩约 34×）。 | Rust 侧 `view` 命令；工具层参数校验 |
| `[x]` | [定居点名字 key](notes/settlements-lazy-table.md) | `Settlement` 改按名字引用，投影新增懒表 `settlements`，测试全绿。 | — |
| `[x]` | [统一总结指标](notes/unified-metrics.md) | 总结指标由步进中间量聚合，agent 视图与 `--digest` 同源、不再重算。 | 治理中间量并入 `RoundView`；Web 是否复用待定 |
| `[~]` | [Step 中间量清单：36 条算完就扔的量](notes/step-intermediates.md) | 数据面下一批：把 `step_*` 里只活在栈上的中间量（忠诚为何在掉 / 批了钱为何没花 / 我为何打不中 / 这单为何没人接）捕获进 `RoundView`。36 条逐条带 `文件:行号`（已在 `main` = `7e11d32` 上复核）+ 粒度 + 是否吃骰子 + 能回答什么问题，分 A 经济治理 / B 市场运输 / C 军事外交三组。**B1（治理/忠诚）已落地**：城行 `loyalty_target` 三项分项、势力行行政/娱乐拆分 + 人口超载倍率 + 两个全国项、`decisions.capital` 稀疏迁都判定（**形状修订见 §6.2**；`SCHEMA_VERSION` 16）。**B2（钱去哪了）已落地**（§6.3）：势力行 `investment_spent`/`construction_spent`/`upkeep_unpaid`/`fleet_rust`、城行 `labor`/`housing_capacity`/`is_hub`/`build`（造舰是缺钱还是缺产能）；「批了多少」**留在控制面**、读面只记已花（相减 = 没花掉的）。**B3（市场与运输）已落地**（§6.4）：`view.market_trades`（一笔成交一行：价格分解 + 丢货率）、`view.haul_steps`（一舰一行：`loaded`/`delivered`/`waiting`/`en_route`——后两档**既不落 state 也不发事件**）、势力行的购买力/买方名次、逐货栈运力账、禁运从计数升级成「名单 + 三档原因」。**B4（战斗）已落地**（§6.5，**用户裁决 Q1 = 进事件层**）：`GameEvent::Attack` 长出 `shots`（逐发：选择三项分 + `hit`/`def_mult`/`pd`/`absorbed`/`soak`/`armor_soak`/`hull_pen`/`damage`/`killed`/`skipped`），**0 伤害的齐射也发**（被点防吃光此前一条事件都不留；同批给 `relations` 的交火判据加显式闸 ⇒ 世界逐字不变）；轨迹行**一个字节没加**（事件只内联 id）；kit 新增 `q.salvos()`。四批都是 digest 逐字段验中性 + 全档绿（B4 后 233 绿 / `SCHEMA_VERSION` 20）。**B5（输入面 `pre`）B5a 已落地**（§6.6）：`pre` 的类型换成 `RoundInputs`——**用户裁决**「凡是可能未来与随机/输入有关的东西都放 `pre`，不一定要求当前的实现有关」，同批**砍掉它原来装的那份零信息量的观测副本**（实测：`pre` 的观测与**上一回合 `post`** 逐字段相同，要读旧世界读上一行）；已接 **C7 逐舰解算顺序** + **C13 关系噪声**（新派生表 `idx/round_inputs.jsonl`、`--derived` 的 `pre`、kit 的 `q.round_inputs()`、`advance_round` 交出输入面而 `advance` 保持原签名）；digest 逐字不变、238 绿、`SCHEMA_VERSION` 21。 | B5b/B5c（`derived_roll` 家族 **实测 ~18 处** + 判定时看到的输入；接入形状已定：骰子由调用方掷、只在拍板处记一条）；**§7 剩下的设计点**（Q2 已裁决落地；Q3 体积已由 `dense-face-sparse-store.md` §8/§9 结掉大半） |
| `[x]` | [稠密读面 / 稀疏存储](notes/dense-face-sparse-store.md) | 用户提的想法（对外稠密、底层自动稀疏）+ 由此量出来的两处浪费。**中性值所有权（§7）已落地**：`src/model/neutral.rs` 一处声明读面每个叶子字段的缺省值，引擎运行时缺省用同一批具名常量，`schema.json` 发 `neutral` 段，五条守卫（含 schemars 双向集合相等 ⇒ 加字段不加声明就红）；kit 的 `q.neutral()` 读同一份声明。**通用稀疏层 §8 裁决为「不做」**，⚠ **§9 在 B2 之后把量化依据重测了**：早先写的「能省的只剩 0.3% / 中性值约占 2%」**是错的**——实测中性值占字段出现次数的 43–47%、过程量按字节占 view 的 25%（3826 B/行）；结论不变，但依据换成了「能省的只有过程量那 25%，而代价是五个读取边界都要 decode」+ 判据本身是坏的（把「没发生」与「恰好是 0」算成一类）。 | 两处浪费已改用约定收掉（见 `step-intermediates.md` §6.2）：`capital` 进稀疏判定数组、两个全国项只存势力行（`main.jsonl` 18127 → 15652 B/行）；encode/decode 与 `--dense`/`--raw` 明确不做 |
| `[x]` | [权威 schema 贯彻](notes/wysiwyg-resource-keys.md) | 资源 key 统一成中文可读名、删掉镜像结构，视图直用权威类型。 | 派生字段要 agent 现场 jq 计算（或加语义视图） |

---

## 工程 · 布局 · 测试

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [CLI 读面：精简 + `--quiet` + 裸调用打 help](notes/cli-surface.md) | 判据 **「这东西 `--index` 投影里有没有？」**：有 ⇒ CLI 上的专用 dump 开关就删。删掉 `--traj`（自包含 story pack，无活调用者）、`--story`（编年史在每条投影行里）、`--notables [N]`/`--milestones [N]`（两层历史在 `events` 的 `salience` 列里，且后者按判据恒为空）、`--rounds` 别名（全仓库 0 处）；顺带删掉 main 里 94 行只为它们存在的渲染代码。新增 **`--quiet`**（`--round N` 只推进不吐轨迹：1000 回合 40.3 MB → **0 MB**，6.3 → 5.2 s）。**裸调用 `planet_x` 现在打 help（exit 0）**；`--seed 42` 这种「有参数没动作」仍走机器可读的 `ERR_USAGE`（exit 10）。行为中性用自带判据验：`--seed 42 --round 240 --digest 20` 的 sha256 逐字节不变。 | `--control-plan`/`--derived`/`--digest`/`--index` 等 11 个都留（各有活调用者）；没上子命令（11 个 flag 不值得）；`agent::story_value` 这个 3 行 wrapper 也一起删了（删掉 `--story` 后 0 调用者） |
| `[x]` | [代码布局：大文件拆小 + 单测搬出源码](notes/code-layout.md) | `sim.rs` 6341 → 170 行 `mod.rs` + 17 个子模块、`control.rs` 3863 → 85 + 8 个；18 个源文件的内联单测全搬到 `src/tests/`（`#[path]` 引入 ⇒ **零可见性放宽**）；纯搬运，digest 逐字节不变。 | 下一轮候选：`model/event.rs` 1141、`projection.rs` 1063、`model/game_config.rs` 992、`world.rs` 865、`autocontrol/shipbuilding.rs` 746 |
| `[x]` | [测试按模拟时间分档](notes/test-tiers.md) | 用 `cargo nextest` 的 group/profile 按**推进回合数**分档：快档 178 条 / 4 s（原 110 s）、中档 184 / 30 s、全档 189 / 96 s；档位写在模块名 `horizon_mid`/`horizon_long` 里，加用例不用改配置。 | 读面契约用例（80–120 回合）仍留在快档的取舍与升级路径见该篇 §6 |
| `[~]` | [测试墙钟：热点清单与待办](notes/test-wall-clock.md) | 测试走的是 dev 档（`opt-level = 0`）⇒ **P0 已加 `[profile.test] opt-level = 2` 并实测**：最重的长局 77.2 → **18.1 s（4.27×）**、快档 4.4 → 2.0 s、中档 29.7 → 8.0 s（全档只推算没实测）；行为中性的两条依据见该篇 §0.1。另附一份「每回合被重复算多次」的热点清单（带文件名/函数名，行号已删——见该篇 §2 开头）。 | P1 纯去重（`faction_power_share` 一回合约 10+ 次、索敌内层逐候选重算）／P2 深缓存／P3 测试侧改读 `advance` 返回的 `RoundView`；全档重测 + P1 的 digest 对账 |
| `[x]` | [读面统一：只有 pre 和 post](notes/pre-post-unify.md) | 派生数据不再分 `flow`+`metrics` 两段，一回合只有**一份视图** `RoundView`（`pre`/`post` 同形）；`RoundFlow` 退成引擎内部的写入口袋 `RoundSink`；`--derived`/`--index`/轨迹/web 三处读面一起换名，`SCHEMA_VERSION` 13→14。⚠ B5 之后 `pre` 已是**输入面**（`RoundInputs`）、`post` 是**结算面**——「同形」这条前提作废，见 `step-intermediates.md` §6.6。 | 数据面（中间量捕获）的下一批见 `notes/pre-post-unify.md` §5 |
| `[~]` | [测试全搬的施工图：判据缺什么数据就往序列化里装什么](notes/test-migration-backlog.md) | 用户 2026-10 裁决：**目标是全搬**——判据缺的数据就装进序列化。现状 **171 条 Python 判据 / 205 条 Rust 用例**（g1 43 / g2 79 / g3 26 / g4 23），剩下的按「缺什么」分四类：**A 数据没序列化**（第 1–3 批已落地：`ships.载货`/`cargo_capacity`、逐资源货栈 `depots`；集货腿 `haul_lanes` **否决**——§5.4）、**B 纯函数**（第 5 批 `planet_x --call <fn>` 已落地）、**C 合成场景**（第 6 批已落地：`_harness.scenario_apply` 走引擎自己的 `--apply` 拨控制叶，4 条用例 / 17 条判据进 g2）、**D 内部契约/错误路径**（`--apply` 报错、`migrate` 分支、中性值表 ⇒ **故意不搬**，清单在 §4）。§5 排了七批的执行顺序，每批都要证明装数据那次**行为中性**（digest 逐字不变）。第 6 批已同步 `main`、四道门全绿，正在合流。 | 施工图 §0.1 / §5.6 / §6.5 |
| `[~]` | [测试与二进制解耦：数据级断言 + 轨迹复用](notes/test-decoupled-suite.md) | **Rust 侧只留搬不走的了**（`feature/test-migrate-rest`）：`play/tests/` 四组共 **120 条**判据（g1 33 / g2 46 / g3 26 / g4 14），命中缓存后 **约 4 s** 跑完（含 7 seed × 1000 + 3 seed × 400 回合）。两轮共搬出 **16 条** Rust 用例（长局五条不变量、确定性、同回合复垦/选装、`projection_derived` 6 条、`control_read_face` 1 条、编年史 2 条、战争最长/最短回合那一半），并**丢掉 3 个过时探针**（被 `probe_multipolar` 取代的两个 + 一次性调试器 `probe_zombies`）。缓存两级：**投影**（指纹 = 二进制 + config ⇒ 自动失效）+ **摘要**（失效键 = 抽取逻辑的代码指纹 ⇒ 只改断言不重算）。⚠ 方案 §4 的 Stage 1（进程内 `OnceLock`）**不成立**：`cargo nextest` 是 process-per-test。另修了 pandas 的 1 ULP 浮点解析（kit 加 `precise_float=True`）。 | §10.5 待办：`--index` 要不要加表过滤（7 seed 缓存 1.2 GB）、要不要把 `run.py all` 写进合流门（要改 `AGENTS.md`）；未搬的只剩 `tests/` 下的探针（只打印不断言）与 `src/tests/**` 的快档单测 |
| [程序化行星参数化](notes/planet-visual-params.md) | 外观从 config 的 body_kinds[].params 驱动；变体即着色器分支 |
| [GLSL 拆成独立文件](notes/glsl-files.md) | `#include <px/...>` 引用关系；反引号坑从此绝迹 |

---

## 快速参考：验证手段

测试按**模拟时间**分档（判据 = 用例真正推进的回合数），细节见
[`notes/test-tiers.md`](notes/test-tiers.md)。分两层，口令也见 [`AGENTS.md`](../AGENTS.md)
的「验证」一节（**合流门 = 两条都要绿**）：

**① 数据级（`play/tests/`，跑在读面上，不用重编；改断言即刻生效）**

- **三组全跑（合流门，缓存命中 ~4 s）**：`uv run --project play/planet_xq python play/tests/run.py all`
- 单组：`… run.py 1`（读面契约，≤60 回合）/ `2`（中组 400 回合）/ `3`（长组 1000 回合 × 7 seed）
- 长局用 `--bin release`（默认）、短局 `--bin debug` 更划算；`--refresh` 无视缓存；
  `-j N` 并行跑几个世界。缓存落在 `target/test-fixtures/`（代码一改自动失效）。

**② Rust 侧（搬不走的那半）**

- **内循环（快档）**：`cargo nextest run` —— T0 + T1（不推进回合 / ≤48 回合）
- **中档**：`cargo nextest run -P mid` —— 加上 T2（49–480 回合）
- **全档（合流门）**：`cargo nextest run -P full` —— 全部非 ignore 用例
- 探针/诊断（只打印不断言，`#[ignore]`）：`cargo nextest run -P full --run-ignored all`
- 没装 nextest 的退路：`cargo test --workspace`（**仍然是全档，慢**）
- 一次简短观察：`cargo run --bin planet_x -- --seed 7 --round 30 --digest 10`（每 10 月一行故事板）
- 一键拿故事素材：`planet_x --seed 7 --round 60 --index out/`，再用 `play/planet_xq`
  (`planet_xq.load('out').facts`) 读主流与 `chronicle`（累计编年史按 `(round,id)` 去重）。
- 纯搬运/拆文件类改动的行为验证：`--seed 42 --round 240 --digest 20` 的 SHA-256 必须逐字节
  不变（取行口径见 [`notes/code-layout.md`](notes/code-layout.md) §3）。
  **当前基线**：`657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665`
  （12 行）——它是「大文件拆分」合并点（`98c4b70`）留下的那条；之后的**读面统一**
  **读面那一路（`main` 这条线）**：`657F2DC9…6665`（= 「大文件拆分」合并点 `98c4b70` 留下的
  那条）被**读面统一**（`feature/pre-post-unify`）、**B1 中间量捕获**
  （`feature/step-intermediates-b1`）、`capital` 形状修订（`feature/capital-decisions`）与
  **B2 钱去哪了**（`feature/b2-money`）逐次验成**逐字节不变**（后三批都是纯结构改动 / 纯追加，
  行为中性）。
  **`feature/tech-system-mond` 这一支**（有意的行为改变，一路换代）：
  `657F2DC9…6665`（= `main`，**M1 连续化**在同一棵树上仍逐字节相同）→ `C726F272…4CBA`（**M2** 加
  「飞船在异常区」渠道）→ `0E3E760D…9A0C`（**特权删掉 + 1.0 棘轮**）→ `A5183C6A…0DC`
  （**崇拜教初始 1.0**）→ `6E376B8F…573F`（**观测编队**：角色轴第三态）→ `7494A2C8…F446`
  （**B1 深空治理**）→ **`81A197493D2EAFF69F02FB03645CF64380CDED87924FA8AEAF482ED911F91811`**
  （**动机自然竞争**：删掉观测上限，改成水位配给）。
  **两条线汇合之后**：基线就是上面最后一格 **`81A197…1811`**——在 `36c4882`
  （tech 支线并入 `main`）与「并入 B2」之后的树上都实测复现；**B2 那一批在合并后的树上同样
  验过它逐字节不变**。⚠ `SCHEMA_VERSION` 两条线都取过 **17**（本支 = `mond_control` 字段、
  读面那一路 = B2 的读面增列）⇒ 汇合后取 **18**，`13..=17` 整段只推号
  （对照表见 `src/model/state.rs`）。
  **`81A197…1811` 之后又变了两次**（都不是本支）：
  **`feature/site-supply`**（`ad93ad2`/`af97de6`，站点自给：非首都投资只吃本地库存；见
  [`notes/site-supply.md`](notes/site-supply.md)）**改了行为却没记基线**——B4 落地时实测
  `main` 已是 **`C928C3F19AFE3BA9D36A70DF8E340E3849271574663920D544AE62AFF70B06A9`**
  （顺手回填在这里）；`feature/b4-combat`（战斗中间量进事件层）在这棵树上**逐字节相同**
  （digest 只多允许 `events` 计数变，实测连计数都没变——见 `step-intermediates.md` §6.5）。
  `SCHEMA_VERSION` 之后又走到 **20**（B3 读面 19 / B4 `Attack.shots` 20）。
  再往前：`657F2DC9…6665`（`main` = `98c4b70`，重构合并点）与更早的重构前（`8b96aef`）逐字节相同，
  那是「纯搬运」的验收证据。
  **`C928C3F1…06A9` 之后又变了**（B5 各批的输入面落地时改过行为、当时没回填）⇒ 在
  `76753c0` 的 `main` 上实测基线已是
  **`975DC8A988F9846330DDCD3845B9C2D37C28D8E6E9893F26AB11DCC912C2E41B`**
  （2026-10 由 `feature/blueprint-stance` 顺手回填；同一份 12 行 JSON 逐字节复现）。
  ⭐ **`feature/blueprint-stance`（删掉舰队默认指令 + 图改带倾向三轴）在这棵树上与 `main`
  逐字节相同**——它的验收证据就是"**默认局面零影响**"：默认局里没人写过那两片叶
  （没有 `--apply`、没有舰队默认、AI 建的图三轴全沉默），所以取值链的化简是**结构性等价**；
  世界行为的变化只发生在"有人真的写了那些叶"的局面上（见
  [`notes/blueprint-stance.md`](notes/blueprint-stance.md) §5）。
  `SCHEMA_VERSION` 之后又走到 **22**（B5 输入面 21 / 本轮 22）。
- ⚠⚠ **那个 `975DC8A9…C2E41B` 复现不出来（2026-10 实测，请下一个 agent 留意）**：在
  `main` 的 `e430532` 与 `0385025` 两棵树上、`release` 与 `debug` 两种档，`--seed 42
  --round 240 --digest 20`（同样的取行口径、12 行）**四个组合都给同一个值** =
  **`C928C3F19AFE3BA9D36A70DF8E340E3849271574663920D544AE62AFF70B06A9`**
  —— 也就是上面那个**更早**的基线。也就是说 `975DC8A9…` 要么来自一份**带本地改动的
  `config/game.ron`**（本仓做 A/B 时有先例：`sed` 拨 `autocontrol.style_chance` 那类开关），
  要么当时量具的口径不同（例如没剥 `\r`、或数了不同行数）。**没法判定谁对**，所以：
  * 本轮及之后的**行为中性验收改用「与 `main` 同机、同 config、同口径逐字节相同」**这条判据
    （它不依赖任何历史记录，可当场复现）；
  * 谁要是能让 `975DC8A9…` 复现，请把复现命令原文补在这里；否则别再拿它当基线。
- ⚠ **别裸跑 `git stash pop`**：这个仓库里躺着**别的分支留下的旧 stash**（当前
  `stash@{0}` = `On feature/military-ships: pre-refactor worktree state`）。它一旦被弹出，会
  把**拆分之前那个 195 KB 的 `src/sim.rs` 单体**复活到工作树（`DU src/sim.rs` 冲突；
  实测踩过一次，用 `git rm -f src/sim.rs` 清掉即可，别去 drop 别人的 stash）。
  要临时关掉一处改动做 A/B，**先 `cp` 备份再用 `sed` 拨那一行**，别用 stash。
