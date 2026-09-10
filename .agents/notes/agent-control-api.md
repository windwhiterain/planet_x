# AI 游玩体验（agent 控制面）

> 状态 `[~]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §8

- `[ ]` **`summary`/`delta` 级联**：把治理/忠诚/MOND/家园防御的语义也放进 agent 的
  `meta` 与每回合 `events`（当前已在 `meta` 暴露 `governance`/`mond`；`events` 已有
  `Revolt`/`Resurgence`）。
- `[x]` **合纵连横格局可见化**：当前每回合 `state` 新增 `coalition` 字段（`hegemon` /
  `members` / `power_share`），`meta` 新增 `balance` 配置——AI 可直读「谁在出头、
  谁在联合制衡、谁被制裁」。
- `[x]` **可读的「忠诚/治理」可视化**：每座城的 agent 输出已带 `loyalty`（0..1）与
  `gov_distance`（到其统治势力首都的距离 AU）——AI 一眼看出哪些城在失稳边缘，好据此
  投娱乐预算 / 决定是否放弃远端殖民地。
- `[ ]` **`control` 面向目标的模板**：给 agent 的 `control` 面再加一个「按目标批量生成
  diff」的辅助命令（例如 `order-fleet-to-hold <body>` / `invest-loyalty <city> <budget>`），
  降低 agent 手写嵌套 JSON 的成本。
