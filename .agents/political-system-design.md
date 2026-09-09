# 《行星X》政治系统设计：议题—立场—关切度 + 三因素关系模型

> 状态：**`[ ]` 未实现**。本设计把「世界级议题」作为给权力关系提供**目的**的核心抽象，
> 并用 **历史(静态) / 思潮(可变) / 利益(实时)** 三因素驱动两两关系，让国际关系
> 「有目的、有历史、有张力」，因此能被 AI 政治操纵、也能自然生成剧情。
> **MOND（行星X崇拜教掌握修正引力）是第一个议题实例。**
>
> 目标：替换 / 增厚 `step_diplomacy` 的单动机漂移，但**保留** `war_threshold` /
> `war_fatigue` / `hostility_floor` / `friendship_ceiling` 与 `step_balance_of_power`
> 作为有界、自平衡的底层。全部新机制**确定性、数据驱动**（`config/game.ron`）。

---

## 0. 为什么现行政治单薄（诊断，已核实代码）

- `Faction`（`src/model.rs` L839）只有 `alignment`（意识形态标量）+ `aggression`，
  另有本土防御。**无目的 / 无立场 / 无历史 / 无特殊关系**。
- `step_diplomacy`（`src/sim.rs` L2058）的关系 = **单动机漂移**：
  向 `|Δalignment|` 推出的「静息亲和」单调收敛 + 战争疲态 + 噪声。**无记忆的马尔可夫**。
  （`config/game.ron` L40 `diplomacy`：`affinity_floor=-42 / affinity_span=70 / drift_rate=0.02`。）
- 均势（`step_balance_of_power` L2398）是**独立第二层**，与意识形态漂移基本无关。
- 结果：AI 没有「为什么」，关系没有「历史」，两者都不反映进政治操纵 → 政治单薄。

---

## 1. 三因素关系模型（核心）

```
rel(a,b) = clamp(
    w_hist     * HIST(a,b)      // 历史：静态（准静态）
  + w_world    * WORLD(a,b)     // 思潮：可变（中速率）
  + w_interest * INTEREST(a,b)  // 利益：实时（每回合重算）
  + w_balance  * BALANCE(a,b)   // 均势（现有 step_balance_of_power）
)
```

| 因素 | 时间尺度 | 来源 | 语义 |
|---|---|---|---|
| **HIST** | 静态 / 准静态 | config `special_relations`（基座）+ `hist_memory`（慢变积累） | 开局给定；只在重大历史事件（战争/条约/共胜）时**跳变**；**不**每回合从当前利益重算 →「静态」 |
| **WORLD** | 可变（中速） | `alignment` / `aggression` 的**思潮位** | 随世界议题相位 + 事件**漂移** →「思潮可以变化」 |
| **INTEREST** | 实时（快） | 当前物质盘面（资源/安全/通路/威胁） | 每回合按利益重算 |
| **BALANCE** | 现成 | `step_balance_of_power` | 独立第四项，保持不变 |

> **关键**：三因素在**不同时间尺度**上贡献同一个关系。它们分别回答：我们**过去**怎样（历史）、
> 我们**信什么**（思潮）、现在**合不合算**（利益）。让「美国对俄罗斯」既可能因历史积怨而冷、
> 又可能因当前共同威胁而暖——张力来自**多因素叠加**，而非单一标量。

---

## 2. 议题—立场—关切度（通用框架）

每个**世界级议题** = 一个有争议、需各势力表态的世界问题：
- `id`、`question`（框架化叙事）。
- **若干立场轴**（每个 `[-1,1]`）。
- 每方在该轴的 **立场位 `stance`** 与 **关切度 `salience`**（0..1）。

某方在轴上的立场 = 意识形态分量与物质分量的**混合**：

```
stance(a, axis) = λ_ideol(a,axis) · ideol_pos(a,axis)
                + (1 − λ_ideol(a,axis)) · interest_pos(a,axis)
```

`λ_ideol`（越按意识形态还是越按物质）由该势力 `worldview.values` 决定 →
**决定该议题的贡献主要进入 WORLD（思潮）还是 INTEREST（利益）**。

### 位置分歧 与 零和竞逐 的分离（用户裁决 #2）

同一议题对**同一对关系**的贡献 = 位置一致项 − 零和竞逐项：

```
issue_contribution(a,b,issue)
    =  Σ_axes  w_pos · 位置一致度(axis)        // 立场接近 → +（共同威胁把大家拉近）
     − Σ_contests w_rivalry · rival_intensity(prize)  // 都要同一排他奖品 → −（抢蛋糕把大家拆散）
```

- **位置一致**：双方在该轴立场接近（`1 − |Δstance|/2`）→ 加分。
- **零和竞逐**：双方**都高关切同一个排他奖品**（赛道重合、奖品独占）→ 减分。
  `rival_intensity = salience_a · salience_b · exclusivity(prize)`；`exclusivity` 来自
  资源稀缺 / 通路独占（如 MOND 掌握、柯伊伯 access）。

> **这是现行系统无法表达的**：现在只有一条 `affinity` 漂移，同一关系永远只朝一个方向走。
> 而真实政治是「**共同恐惧把我们拉近，同一块蛋糕把我们拆散**」——两股**相反**压力在同一条
> 关系上共存，这就是把政治做厚、做得有机的核心。

---

## 3. MOND 议题（第一个实例）

议题 `mond`：「柯伊伯与引力之道」。立场轴：

- `contain_vs_race`（多边遏制 ⟷ 单干竞逐）
- `threat_vs_tolerate`（生存威胁 ⟷ 可容忍）

各势力立场（由世界观导出，确定性设定，示例）：

| 势力 | `contain_vs_race` | `threat_vs_tolerate` | λ_ideol | 语义 |
|---|---|---|---|---|
| 行星X崇拜教 | 极正（神圣独占） | 正（拥抱使命） | 高 | 它就是那「奖品持有者」 |
| 无国界科学组织 | 极负（多边/开放） | 正（弄清并共管） | 高 | 想共享、防垄断 |
| 中国/美国/俄/欧 | 正（单干竞逐） | 正（怕教团控外缘） | 中 | **既抱团又互斗** |
| 星系矿业 | 只看「谁说了算对我有利」 | 负（可利用、借道牟利） | 低 | 投机逐利 |
| 深空运输联盟 | 只看「谁控航道对运输有利」 | 负（可容忍） | 低 | 投机逐利 |
| 联合国 | 极负（多边） | 正（理应集体应对） | 高 | 多边秩序 |

由此得到的图谱（**确切复现用户的种子**）：
- **大国—大国**：`threat_vs_tolerate` 一致(+) 被 `contain_vs_race` 的**零和竞逐**(−) 对冲
  → **若即若离**——「各国恐慌(共同威胁) + 互相猜忌谁先搞定(战略竞逐)」。
- **大国—联合国/科学组织**：`threat_vs_tolerate`(+)、且无零和竞逐 → **合作**。
- **矿业/运输—教团**：可容忍(+) + 经济投机，被大国视为「通敌/绥靖」→ 与各大国在
  `threat_vs_tolerate` 上**分歧** → 被疏远。

> **MOND 的「零和竞逐」信号已机械存在**：`config/game.ron` `mond.masters = ["行星X崇拜教"]`
> （只有它在异常区不迷航）+ `mond_control`（见 §7）。教团独占外缘、别人要抢 →
> 就是大自然给「零和奖品」的物理载体。

---

## 4. 思潮动力学（可变）

`alignment` / `aggression` 从**静态身份**变为**可漂移的思潮变量**（当前是死值，这是关键改动）。
漂移受三类推动，**确定性**、无未播种 RNG：

- **世界议题相位**（§7）：如 MOND 恐慌 → 大国对 `threat_vs_tolerate` 的思潮关切抬升、
  `alignment` 向「反教团」漂移；峰会/缓和 → 向合作漂。
- **事件**：战争 → `aggression` 上；长期和平 → 回落；条约/共同胜利 → `alignment` 向共识靠。
- **内部冲击**（可选，后续）：灾变 / 内部动乱改变激进/保守。

> 思潮可变的收益：势力身份**不是铁板**——同一个国家会因世界剧变而变得更鹰或更鸽。
> 这是「思潮可以变化」的直接体现。由于 `step_diplomacy` 读 `fa.alignment`，这个漂移会
> 自然传导进现有 `affinity` 公式，改动最小。

---

## 5. 利益实时计算

`interest_pos(a, axis)` 由**当前**物质盘面算出（不是世界观，是盘面）：
- **柯伊伯 / 资源通路**：`mond_control`（MOND 掌握度）、是否 `masters`、某关键矿产/航道依赖。
- **安全**：邻国威胁、本土防御紧张度（`home_radius` 重叠、舰艇威慑对比）。
- **资源经济**：稀缺矿产库存、贸易依赖、市场价。

`rival_intensity(a,b,prize) = salience_a · salience_b · exclusivity`。双方都对同一排他
奖品高关切 → 大幅 **−**。

> 利益项是**每回合重算**的「实时因素」；它让 AI 的立场随盘面变（缺铁就追铁、失守就警惕），
> 而不是被固定身份锁死。

---

## 6. 历史：config 基座 + 路径依赖记忆（用户裁决 #4「都要」）

### 6.1 特殊关系基座（静态历史，开局即读得懂）
`config/game.ron`：
```rust
special_relations: [
  { a: "中国", b: "俄罗斯",   base: +0.9, kind: Alliance,  note: "背靠背，MOND 立场相近" },
  { a: "美国", b: "欧盟",     base: +0.7, kind: Alliance,  note: "大西洋同盟" },
  { a: "美国", b: "中国",     base: -0.3, kind: Rivalry,   note: "MOND 竞逐 + 意识形态对立" },
  { a: "联合国", b: "行星X崇拜教", base: -0.8, kind: Grievance, note: "世仇：教团否弃世俗秩序" },
  { a: "星系矿业", b: "行星X崇拜教", base: +0.2, kind: Contested, note: "柯伊伯资源争夺，亦敌亦商" },
  { a: "深空运输联盟", b: "无国界科学组织", base: -0.2, kind: Dispute, note: "航道 vs 科研频段之争" },
]
```
开局关系**非均匀、有意义**——这个世界开局的政治地图读一遍就懂（现在所有关系都从 0 起、
由 `|Δalignment|` 唯一决定，开局没有地缘形状）。

### 6.2 路径依赖记忆（慢变积累）
每对关系加一个**慢变基线** `hist_memory`：战争/背叛 → 重伤（积怨）；条约/随罪同抗/共同胜利
→ 抬升（恩义）。比议题快层动得慢，所以**任一时刻近似「静态」**——它不追当前利益，只沉淀过去。

---

## 7. MOND 作为组织性危机 + 相位（剧情接口）

把 MOND 从静态 `masters` 升级为**驱动世界的议题**：`mond_control`(per faction) 追踪掌握度
（教团高、他方 0 起、可经研究/掠夺/交易上升）。世界按**确定性阈值**过相位：

```
觉醒(cult 掌握曝光 → 全球恐慌)
  → 竞逐(大国抢 MOND → 零和竞逐项上冲 → 大国渐冷、但共同威胁仍高)
  → 摊牌/峰会(降温) 或 冲突(开战)
  → 余波(均势 step_balance_of_power 重新接管)
```

**相位切换 → 重设思潮关切度 + 触发剧情节拍**。于是：
- **政治生成剧情**：相位 / 霸权 / 联盟 / 制裁 / 竞逐转折本来就是剧情的高潮素材。
- **剧情塑造政治**：剧情节拍的效果把思潮 / 关切度 / 立场推向下一相位。
- 两者闭环——正是「国际关系与剧情更有机结合」的机制落点。

---

## 8. 世界观 → 大战略 → 政策（给 AI 目的）

- **世界观（恒定身份）**：`worldview { ends, values }`，`values` 对
  security / expand / economy / knowledge / order 的权重。决定它「执着于什么」、
  各议题的默认立场与 λ_ideol。
- **议题立场/关切度（动态）**：由 `values × 当前议题局势` 算出。
- **大战略（AI 目的）**：世界现在什么局势、我最在乎什么 → 选对外**政策**：
  - 集体遏制 / 自主竞逐 / 自由科学 / 经济投机 / 调解峰会。
  - 每种政策 = 一组对动机的加权（抬哪些关系、压哪些关系）。

> AI 不再「按意识形态亲疏冷漠漂移」，而是**为了推进它在某议题上的立场，而去拉拢/孤立
> 特定势力**——关系成了手段，立场成了目的。这就是「AI 有目的、会政治操纵」。

---

## 9. 数据模型（`config/game.ron` 示意）

```rust
// 思潮（alignment/aggression 的动态化参数）
factions: { "中国": ( alignment: +0.1, aggression: 0.5, worldview: (
    ends: "战略自主，防被锁在柯伊伯带外",
    values: { security: 0.6, expand: 0.5, economy: 0.5, knowledge: 0.3, order: 0.4 },
), ... ) }

// 世界级议题（通用框架；MOND 为第一个实例）
issues: {
  "mond": {
    question: "柯伊伯与引力之道：谁该掌控、如何对待教团的 MOND 掌握",
    axes: {
      "contain_vs_race":    { label: "多边遏制 ⟷ 单干竞逐", default_pos: {...} },
      "threat_vs_tolerate": { label: "生存威胁 ⟷ 可容忍",   default_pos: {...} },
    },
    contests: [ { prize: "kuiper_mond_access", drives: "contain_vs_race" } ],
    phases: [ { at: mond_control_threshold_world, ... } ],
  }
}

// 关系分解权重
relation_model: { w_hist: .., w_world: .., w_interest: .., w_balance: ..,
                  rivalry_gain: .., rivalry_floor: .. }
```

---

## 10. 机制步进（step ordering，`sim.rs::advance` L61）

```
step_ideology        // 思潮漂移（alignment/aggression 动态化）
step_interest        // 利益重算（mond_control / access / 安全 / 资源）
step_relations       // 三因素合成（替代/增厚 step_diplomacy）
step_balance_of_power// 均势（不变）
step_mond_crisis     // 相位推进（重设思潮关切度 + 剧情触发）
step_story           // 剧情（晚于政治，能读最新政治状态；写的关系变化下一轮反馈）
```

故事晚于政治 → 能读到**刚算出的**霸权/联盟/制裁/竞逐状态；故事写的思潮/立场/关系变化
**下一轮**经 `step_ideology`/`step_interest`/`step_relations` 反馈——**天然闭环**，顺序现成。

---

## 11. Agent 暴露（政治操纵面）

- 每势力：`worldview` / `positions`(各议题立场) / `policy`(当前大战略)。
- 每议题：当前 `stance 格局`（谁极端 / 谁骑墙 / 谁缺席）。
- 每对关系：`motives { hist, world, interest, balance }` 分解 + `history`（近期积怨/恩义）。
- 控制叶子：设 `policy`、发起 `treaty/summit`(对某议题降温)、直接改立场/关切度。

> 让 AI 能**看懂**「为什么冷/为什么抱团」并**操纵**它——政治不再是黑盒，而是可读、可插手。

---

## 12. 剧情接口（衔接「国际关系 × 剧情」）

复用上次讨论的「反应式触发 + 生成式编年史官 + 结构性效果」：
- **反应式触发**：`HegemonRises` / `CoalitionEstablished` / `Sanctioned` / `HegemonFalls` /
  `AllianceFormed/Broken` / `Escalation/Deescalation` / 议题相位切换。
- **生成式编年史官**：每回合从 `balance_picture`/`sanctioned_hegemon`/`war_pairs`/议题相位
  合成一行「现场播报」，命名真实玩家，永远与数字一致。
- **结构性效果**：`DeclareWar` / `ForcePeace` / `SetRelation` / `OstracizeAllAgainst` /
  `ImposeSanctions` / `FormCoalition` / `FactionGoal`——剧情**真的驱动政治**，而不只是 nudge。
- `story_participants`（`sim.rs` L2532）已支持按事件动态填充参与方 → 剧情节拍天然自描述。

---

## 13. 平衡与守卫（不破坏「上千回合仍多极」）

- **保留** `war_threshold` / `war_fatigue` / `hostility_floor` / `friendship_ceiling`
  作有界底层。
- 新增关系**必须 clamp 有界**（防极端敌对/友好）；思潮漂移有界；零和竞逐加 `rivalry_floor`
  防过度敌对；`exclusivity` 依资源稀缺/通路独占，防止凭空抬高冲突。
- 长局守卫需全过：`world_is_multipolar` / `no_nonfinite_over_long_run` /
  `world_value_is_bounded` / `max_dead ≤ 2` / `same_seed_reproduces_identically`。
- 校准用 `diagnose_long_horizon`（3 种子 × 3000 回合）。

---

## 14. 实施顺序（MVP 切分，均独立可验证）

- **M1（低风险，不碰均势）**：议题框架 + 特殊关系基座 + 关系**动机分解可视**；
  关系 = 现有漂移 + `HIST` + `INTEREST` 的**位置一致项**（暂不加零和项）。
  验证：`world_is_multipolar` 不退化、关系分解正确。
- **M2**：思潮漂移（`alignment`/`aggression` 动态化）+ 世界观/大战略/政策 → **AI 有目的**。
- **M3（最大）**：零和竞逐项 + MOND 议题动态化（`mond_control` + 相位 + 剧情接口）
  → **组织性危机**，也是「政治×剧情」闭环的核心。

---

## 15. 待定（需拍板）

- 思潮漂移的**驱动源清单与强度**（哪些事件/相位推哪股思潮、速率）。
- 立场位 `λ_ideol` 意识形态/物质混合的**默认值与校准**。
- 零和竞逐的 `exclusivity` **从哪来**（资源稀缺？`mond_control` 独占？通路？）。
- **M1 能否不引入思潮漂移就独立落地**（先让「历史+利益位置项」有界、可验证）。
