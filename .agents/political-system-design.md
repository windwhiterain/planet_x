# 《行星X》政治系统设计：议题—立场—关切度 + 三因素关系模型

> 状态：**`[ ]` 大部分未实现**；其中「思潮(Ideology)最小功能实验」已落地（branch `feature/ideology`，
> 见 ideas.md）。本设计把「世界级议题」作为给权力关系提供**目的**的核心抽象，并用
> **历史(静态) / 思潮(可变) / 利益(实时)** 三因素驱动两两关系，让国际关系「有目的、有历史、
> 有张力」，因此能被 AI 政治操纵、也能自然生成剧情。**MOND（行星X崇拜教掌握修正引力）是第一个
> 议题实例。** 目标：替换 / 增厚 `step_diplomacy` 的单动机漂移，但保留
> `war_threshold`/`war_fatigue`/`hostility_floor`/`friendship_ceiling` 与 `step_balance_of_power`
> 作有界底层。全部新机制确定性、数据驱动（`config/game.ron`）。

---

## 0. 为什么现行政治单薄（诊断，已核实代码）

- `Faction`（`src/model/faction.rs`）只有 `alignment`（意识形态标量）+ `aggression`，另有本土防御。
  无目的 / 立场 / 历史 / 特殊关系。
- `step_diplomacy`（`src/sim.rs`）的关系 = **单动机漂移**：向 `|Δalignment|` 推出的「静息亲和」
  单调收敛 + 战争疲态 + 噪声。**无记忆的马尔可夫**。
- 均势（`step_balance_of_power`）是独立第二层，与意识形态漂移基本无关。
- 结果：AI 没有「为什么」，关系没有「历史」，政治单薄。

---

## 1. 三因素关系模型（核心）

```
rel(a,b) = clamp( w_hist*HIST + w_world*WORLD + w_interest*INTEREST + w_balance*BALANCE )
```

| 因素 | 时间尺度 | 来源 | 语义 |
|---|---|---|---|
| **HIST** | 静态 | config 特殊关系基座 + 慢变记忆 | 开局给定，只在重大历史事件时跳变；每回合不从当前利益重算 |
| **WORLD** | 可变（中速） | `Ideology` 4 条思潮轴 | 随世界议题相位 + 事件漂移（**思潮可以变化**） |
| **INTEREST** | 实时 | 当前物质盘面 | 每回合按资源/安全/通路/威胁重算 |
| **BALANCE** | 现有 | `step_balance_of_power` | 独立第四项 |

> 关键：三因素在**不同时间尺度**上贡献同一个关系，分别回答「过去怎样 / 信什么 / 合不合算」。

## 2. 议题—立场—关切度（通用框架）

每个**世界议题** = 一个有争议、需各势力表态的问题：`id`/`question` + 若干立场轴（[-1,1]）+
每方在该轴的**立场位**与**关切度**。某方立场 = `λ·ideol_pos + (1−λ)·interest_pos`（`λ` 由
`worldview.values` 定，决定该议题贡献主要进 WORLD 还是 INTEREST）。

**位置分歧 与 零和竞逐 分离**：`issue_contribution = Σ位置一致度 − Σ零和竞逐烈度`。
位置一致（共同威胁）拉近；零和竞逐（都想要同一排他奖品）拆散；两者可并存于同一对关系——
这是政治张力的来源。

## 3. MOND 议题（第一个实例）

议题 `mond`，轴 `contain_vs_race`(多边遏制⟷单干竞逐)、`threat_vs_tolerate`(生存威胁⟷可容忍)。
大国：`contain_vs_race` 正（单干竞逐）、`threat_vs_tolerate` 正（怕教团控外缘）→ **既抱团又互斗**。
科学组织/联合国：多边+共管 → 与大国合作。矿业/运输：可容忍+投机 → 被大国视为绥靖。
MOND 的零和竞逐信号已机械存在：`config/game.ron` `mond.masters=["行星X崇拜教"]`。

## 4. 思潮动力学（可变）

`Ideology`：和平↔军国 / 科学↔技术 / 人民↔精英 / 自然↔殖民，各 [-1,1]。已落地最小版
（`src/sim.rs::step_ideology`），变化因素：①战争得失 ②飞船在 MOND 区 vs 开采 MOND 区资源
③经济净流 ④人均面积。目前只是可读状态。

## 5. 利益实时计算 / 6. 历史 基座+记忆

（待设计细化——基座 = config `special_relations`；记忆 = 战争/条约/共胜积累。）

## 7. MOND 作为组织性危机 + 相位

`mond_control`(per faction) 追踪掌握度，世界按确定性阈值过相位：觉醒→竞逐→摊牌/峰会后余波；
相位切换重设思潮关切度 + 触发剧情节拍（政治生成剧情、剧情塑造政治）。

## 8. 世界观 → 大战略 → 政策 / 9. 数据模型 / 10. 步进 顺序

（待细化；`advance` 已插入 `step_ideology`，故事晚于政治。）

## 11. Agent 暴露 / 12. 剧情接口

思潮经 `Faction.ideology` 自动暴露（`--round`/`--traj` 复用权威 `Faction`）；`--index` 投影加
`ideology` 列。剧情接口（反应式触发+生成式编年史官+结构性效果）沿用上次讨论。

## 13. 平衡守卫 / 14. 实施顺序 / 15. 待定

思潮最小版不碰均势，长局守卫全过（`world_is_multipolar` 等 6 项）。M1=议题框架+基座+动机分解；
M2=思潮漂移（**已落地**）+大战略/政策；M3=零和竞逐+MOND 相位。待定：思潮漂移速率、`λ_ideol`、
零和`exclusivity` 来源、M1 是否独立落地。
