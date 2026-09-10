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
| `[~]` | [贸易与制裁](notes/trade-and-sanctions.md) | 市场已改成**真实交换所**（挂单记名卖家/稀缺定价到 8×/按挂单配给/关系即价格/全面禁运），组件硬门槛让稀缺咬到战斗力，**运费+MOND 承运**让崇拜教垄断柯伊伯带货运，并**删掉凭空重建**（重建改为「派船去复垦」）。 | 平衡待调（整合到 3-4 家）；贸易结构出口端未做 |
| `[~]` | [运输：舰船行为 + 集货 + 雇佣运力市场](notes/freight-collection.md) | 公理「**首都即集散地**」定下对外路线；**运输做成舰船真实行为**：`ShipBehavior::Haul`（常驻路线，腿别由货舱推出）+ 产地货栈（非首都产出必须靠船运回）+ **角色轴**（`Ship.freighter`，像风格一样的三态控制属性，AI 按积压定编、玩家可压住；不影响自动开火/kiting）+ **派单按积压占比抽签**（`derived_roll`，不消费主 Prng 流）。MOND 已按裁决**概率化**：偏移是伪随机范围，深处**没有进不去的目标**，只是要多试几个回合。**雇佣运力市场**（用户裁决）：单子以**雇佣船**的形式，雇主挂单的逻辑与派自己的船同源（自己船不够才挂），单子要求的是**运力（件/回合）**；雇主**周期性考核**受雇方的实测吞吐并反馈信誉（信誉只由考核产生），再按信誉决定续约还是换人；受雇方按自己的运力决定接不接受、要不要提前结束；**船沉没不管，对方派几艘船都无所谓**。 | M0–M2b 与 **M4a–c 已落地**、**M4R（雇佣形态改写）已落地**（本分支 schema 12，**合并设计图分支后 = v13**：两条分支各自从 v9 出发、共用过 v10，见该篇 §7 末）；**A/B 实测：集货把流动经济放大 3.5–4.7 倍**；瓶颈仍是舰队里没有货船（每趟只装 3.4–4.9 件）⇒ Q9。雇佣市场实测（seed 3/7/11 × 400 回合）：搬到位 **917–2011 件**、**实付抽成 15.7–17.3%**（开叫 15%）、考核好评/差评 40/18・29/13・15/34、平均达标率 1.06–1.40（修掉「尺子坏了」之前的 6.10）、关系多数**跑满固定期**（抽手仅 11–22 次） |
| `[~]` | [思潮 → 角色：运输还是战斗](notes/ideology-roles.md) | 角色轴（`ship_freighter`）的**来源**从硬定编改成**思潮驱动**：`尚武度 = 军国 − 殖民`（两轴同权反号、写死）⇒ `倾向 = 2σ(−1.5×尚武度)`（**中庸 = 1.0 = 旧硬定编**，行为中性）⇒ 目标头数 = 有积压的货栈数 × 倾向；派人是**按缺口抽签**（与 `route_for` 同一条纪律：概率分布 = 想要的比例 ⇒ **期望入伙数 = 缺口**、目标处概率恰为 0 ⇒ 不抖），票按运力效率（软排序，最好的船 ≈ 20 倍票）。实测：平均头数贴着配额（±0.06）、配额处头数 ±1 呼吸而人员**一直在换**（0.52 条/回合）；**第一版「每艘舰各自掷一次身份」实测会在 0 与 12 之间两极震荡**。观察面加 `freight_lean`/`freighter_quota`/`freighter_count` 三列（分开「思潮不让跑」与「没人可派」）。 | **角色轴 + 造舰动机解耦都已落地**（本分支）：角色 = 思潮配额按缺口抽签、**轮换**（头数稳、人员流动）；造战斗舰 = **敌对国与自己的实力差距**（连续量，取代「是否处于战争」布尔）；造货船 = **搬不动的比例**（雇得到人就不造）→ 腾**一个**船坞，两条动机各占一个船坞；**AI 会估建造时间**（`build_rounds`，与真实建造同源）——第一版货船舰级只看运力 ⇒ 全世界船坞改成下不了水的航母、600 回合只拆平 4 次，改成「运力÷建造时间」后同一局 37 次。长局发现思潮自己会饱和到殖民端、动机均值偏满（0.79–0.89） |
| `[~]` | [治理与忠诚度](notes/governance-loyalty.md) | 治理开销与忠诚按「距首都 × 人口超载」计费，付不起就离心；已实现城市倒戈换主。 | 「倒戈目标要治理得住」的过滤；忠诚影响生产未做 |

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
| `[~]` | [控制属性 = 活层](notes/control-live-layers.md) | 三态归属 + 势力级默认指令 + 写值即接管 + **风格活层（doctrine/kiting）**全部落地；§4 四条已裁决；**§8 = 控制面板七条裁决**、**§9 = 已确认的动手顺序**。**§10 = web 三条已落地**（`f673bac`）；**§11 = 两轴叶 + 删叶（方案 A）已落地**；**§12 = 角色轴补齐**；**§13 = 读面/写面两侧对齐已落地**（`feature/read-face-parity`：`ship_orders` 读面**每舰一行**、`behavior` 取有效值（`null` = 没人说话）+ 写面「`null` 行不建叶」+ kit 的 `_approx` 换成引擎的 `effective`/`order_source`，`order_*` 回到**真实的叶**）；**§16 = 风格三轴的真执行者**（`autocontrol::style`：按战况概率重估、分布步长、`derived_roll`、三道玩家闸门）、**§17 = `Auto` 设计图的执行者**（`autocontrol::blueprints`：AI 建图/重估/去重/回收，图库 `O(主题×舰级)`）——**§3.2 那条"风格轴没有执行者"的空头承诺已经还清**。 | 两个执行者都还不够平衡（§17.3 的"买不起不下水"扩到 AI 图、玩家图的选装复用、主题权重再标定）；方案 B **已否决**（见 [`control-value-rule.md`](notes/control-value-rule.md)）；文档收尾（把取值规则写全） |
| `[x]` | [控制属性的取值规则](notes/control-value-rule.md) | **裁决：模型没有歧义，不改。** 归属与取值是同一条链上的同一件事（`Inherit` = 没意见、让位给上面的 `Player`；`Auto` 那一档的**值存在逐舰叶里**⇒「叶存在就供值」）。§1 六行实测表逐行由此推出；§3 结清三条"看起来像歧义"的旧账（恢复继承不还值 = 交互落差，出口是删叶；文档只写了归属那一半；势力默认叶 `Auto` 档没有存储）。 | 把 §1 写进 `src/control.rs` 顶部 + `agent-play.md`（文档收尾）；方案 B/B3 已否决 |
| `[~]` | [Lazy 索引分析层](notes/lazy-index-pandas.md) | 重型字段拆成按 id 的懒表，Python/uv 套件读 schema 后 join 分析；**新增 `derived` 段与 `q.derived(...)/q.control()` 等派生表读法**。 | parquet、更多 lazy 字段、剩余统计函数 |
| `[~]` | [长局控制面缺口](notes/agent-control-long-game.md) | 192 月长局实测：预算只能限速不能封顶、无外交/交战规则/放弃城市叶片、结构性叶片所有权不明、幽灵权重。 | §5 新舰默认归 AI 已被 `control-live-layers.md` 解掉；其余全部（§1 维护费上限、§2 ROE 最关键） |

## WebUI

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [自动选空闲端口](notes/web-auto-port.md) | dev server 从 3000 起向上扫空闲端口，`PORT` 三种语义并回显实际 URL，已实机验证。 | `PLANET_X_WEB_HOST` 放开监听地址（安全决定） |
| `[x]` | [服务生命周期](notes/web-lifecycle.md) | 启动者租约 + 关页即退解决孤儿进程锁 exe；`scripts/web.ps1` 成为唯一入口。 | 无存档；浏览器崩溃不自退；Job Object 兜底 |
| `[x]` | [3D 地图渲染](notes/webui-3d-rendering.md) | 用户报的 4 项 3D 观感问题全修完：光照法线、尺度、材质、遮挡。 | 9 舰同屏标记去重；太阳表面粒面与边缘变暗 |

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
| `[~]` | [Step 中间量清单：36 条算完就扔的量](notes/step-intermediates.md) | 数据面下一批：把 `step_*` 里只活在栈上的中间量（忠诚为何在掉 / 批了钱为何没花 / 我为何打不中 / 这单为何没人接）捕获进 `RoundView`。36 条逐条带 `文件:行号`（已在 `main` = `7e11d32` 上复核）+ 粒度 + 是否吃骰子 + 能回答什么问题，分 A 经济治理 / B 市场运输 / C 军事外交三组。**B1（治理/忠诚）已落地**：城行 `loyalty_target` 三项分项、势力行行政/娱乐拆分 + 人口超载倍率 + 两个全国项、`decisions.capital` 稀疏迁都判定（**形状修订见 §6.2**；`SCHEMA_VERSION` 16）。**B2（钱去哪了）已落地**（§6.3）：势力行 `investment_spent`/`construction_spent`/`upkeep_unpaid`/`fleet_rust`、城行 `labor`/`housing_capacity`/`is_hub`/`build`（造舰是缺钱还是缺产能）；「批了多少」**留在控制面**、读面只记已花（相减 = 没花掉的）。**B3（市场与运输）已落地**（§6.4）：`view.market_trades`（一笔成交一行：价格分解 + 丢货率）、`view.haul_steps`（一舰一行：`loaded`/`delivered`/`waiting`/`en_route`——后两档**既不落 state 也不发事件**）、势力行的购买力/买方名次、逐货栈运力账、禁运从计数升级成「名单 + 三档原因」。三批都是 digest 逐字不变 + 全档绿（B3 后 225 绿 / `SCHEMA_VERSION` 19）。 | B4 战斗 → B5 `pre` 面；**§7 三个设计点要先裁决**（逐发索敌计划放哪 / `pre` 面怎么产 / 体积——后者已由 `dense-face-sparse-store.md` §8/§9 结掉大半） |
| `[x]` | [稠密读面 / 稀疏存储](notes/dense-face-sparse-store.md) | 用户提的想法（对外稠密、底层自动稀疏）+ 由此量出来的两处浪费。**中性值所有权（§7）已落地**：`src/model/neutral.rs` 一处声明读面每个叶子字段的缺省值，引擎运行时缺省用同一批具名常量，`schema.json` 发 `neutral` 段，五条守卫（含 schemars 双向集合相等 ⇒ 加字段不加声明就红）；kit 的 `q.neutral()` 读同一份声明。**通用稀疏层 §8 裁决为「不做」**，⚠ **§9 在 B2 之后把量化依据重测了**：早先写的「能省的只剩 0.3% / 中性值约占 2%」**是错的**——实测中性值占字段出现次数的 43–47%、过程量按字节占 view 的 25%（3826 B/行）；结论不变，但依据换成了「能省的只有过程量那 25%，而代价是五个读取边界都要 decode」+ 判据本身是坏的（把「没发生」与「恰好是 0」算成一类）。 | 两处浪费已改用约定收掉（见 `step-intermediates.md` §6.2）：`capital` 进稀疏判定数组、两个全国项只存势力行（`main.jsonl` 18127 → 15652 B/行）；encode/decode 与 `--dense`/`--raw` 明确不做 |
| `[x]` | [权威 schema 贯彻](notes/wysiwyg-resource-keys.md) | 资源 key 统一成中文可读名、删掉镜像结构，视图直用权威类型。 | 派生字段要 agent 现场 jq 计算（或加语义视图） |

---

## 工程 · 布局 · 测试

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [代码布局：大文件拆小 + 单测搬出源码](notes/code-layout.md) | `sim.rs` 6341 → 170 行 `mod.rs` + 17 个子模块、`control.rs` 3863 → 85 + 8 个；18 个源文件的内联单测全搬到 `src/tests/`（`#[path]` 引入 ⇒ **零可见性放宽**）；纯搬运，digest 逐字节不变。 | 下一轮候选：`model/event.rs` 1141、`projection.rs` 1063、`model/game_config.rs` 992、`world.rs` 865、`autocontrol/shipbuilding.rs` 746 |
| `[x]` | [测试按模拟时间分档](notes/test-tiers.md) | 用 `cargo nextest` 的 group/profile 按**推进回合数**分档：快档 178 条 / 4 s（原 110 s）、中档 184 / 30 s、全档 189 / 96 s；档位写在模块名 `horizon_mid`/`horizon_long` 里，加用例不用改配置。 | 读面契约用例（80–120 回合）仍留在快档的取舍与升级路径见该篇 §6 |
| `[~]` | [测试墙钟：热点清单与待办](notes/test-wall-clock.md) | 测试走的是 dev 档（`opt-level = 0`）⇒ **P0 已加 `[profile.test] opt-level = 2` 并实测**：最重的长局 77.2 → **18.1 s（4.27×）**、快档 4.4 → 2.0 s、中档 29.7 → 8.0 s（全档只推算没实测）；行为中性的两条依据见该篇 §0.1。另附一份「每回合被重复算多次」的热点清单（带文件名/函数名，行号已删——见该篇 §2 开头）。 | P1 纯去重（`faction_power_share` 一回合约 10+ 次、索敌内层逐候选重算）／P2 深缓存／P3 测试侧改读 `advance` 返回的 `RoundView`；全档重测 + P1 的 digest 对账 |
| `[x]` | [读面统一：只有 pre 和 post](notes/pre-post-unify.md) | 派生数据不再分 `flow`+`metrics` 两段，一回合只有**一份视图** `RoundView`（`pre`/`post` 同形）；`RoundFlow` 退成引擎内部的写入口袋 `RoundSink`；`--derived`/`--index`/轨迹/web 三处读面一起换名，`SCHEMA_VERSION` 13→14。 | 数据面（中间量捕获）的下一批见 `notes/pre-post-unify.md` §5 |

---

## 快速参考：验证手段

测试按**模拟时间**分档（判据 = 用例真正推进的回合数），细节见
[`notes/test-tiers.md`](notes/test-tiers.md)：

- **内循环（快档，~4 s）**：`cargo nextest run` —— T0 + T1（不推进回合 / ≤48 回合）
- **中档（~30 s）**：`cargo nextest run -P mid` —— 加上 T2（49–480 回合）
- **全档（~96 s，合流门）**：`cargo nextest run -P full` —— 全部非 ignore 用例
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
  **两条线汇合之后（当前）**：基线就是上面最后一格 **`81A197…1811`**——在 `36c4882`
  （tech 支线并入 `main`）与「并入 B2」之后的树上都实测复现；**B2 那一批在合并后的树上同样
  验过它逐字节不变**。⚠ `SCHEMA_VERSION` 两条线都取过 **17**（本支 = `mond_control` 字段、
  读面那一路 = B2 的读面增列）⇒ 汇合后取 **18**，`13..=17` 整段只推号
  （对照表见 `src/model/state.rs`）。
  再往前：`657F2DC9…6665`（`main` = `98c4b70`，重构合并点）与更早的重构前（`8b96aef`）逐字节相同，
  那是「纯搬运」的验收证据。
- ⚠ **别裸跑 `git stash pop`**：这个仓库里躺着**别的分支留下的旧 stash**（当前
  `stash@{0}` = `On feature/military-ships: pre-refactor worktree state`）。它一旦被弹出，会
  把**拆分之前那个 195 KB 的 `src/sim.rs` 单体**复活到工作树（`DU src/sim.rs` 冲突；
  实测踩过一次，用 `git rm -f src/sim.rs` 清掉即可，别去 drop 别人的 stash）。
  要临时关掉一处改动做 A/B，**先 `cp` 备份再用 `sed` 拨那一行**，别用 stash。
