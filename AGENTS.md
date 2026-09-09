# 太空沙盘游戏《行星X》

回合制太阳系沙盘：每回合 = 1 个月，全程数据驱动（`config/game.ron`）、确定性可复现。

- [设计 spec](.agents/spec.md)
- [点子库（活文档，记得回填）](.agents/ideas.md)
- **[agent 游玩手册](.agents/agent-play.md)** ← 想「玩」先读这个

- 不考虑向前兼容

## 给 agent 的工作约定

- **边实现、边想点子**：在做当前目标时，冒出的新机制/新平衡/新剧情只要值得做就**先尝试**；
  做成验证编译通过、长局 harness 不崩、确有增益的改动。
- **来不及实现的写进点子库**：任何想到但这一回合来不及做完/验证的点子，一律**追加到
  [`.agents/ideas.md`](.agents/ideas.md)**（写得够细，让下一个 agent 能照做），别让它在对话
  里蒸发。已实现的勾成 `[x]` 并注明模块，避免重复劳动。

## 想「玩」这游戏：30 秒上手

1. **读世界**：`planet_x --seed 42 --round N --index out/`，然后用 Python kit `planet_xq`
   （`cd play/planet_xq && uv sync`）一行拿决策视图：
   `q.faction_snapshot(r, "中国")` → 该势力的库存/外交/经济/舰队/所属城。
2. **下指令**：写一个 `{control:[...]}` diff（`--apply` 文件），把目标势力/舰/预算钉成
   `"mode":"Player"`，再 `planet_x --start ckpt.ron --apply diff.json --round K --save ckpt.ron`
   （`--start`/`--save` 保 RNG，可复现、可回滚）。
3. **看点子**：详细 loop、可选指令面、坑与边界，全在 [agent 游玩手册](.agents/agent-play.md)。

### 三条最容易踩的坑（先记住）

- **id 永远是字符串名**（舰/城/势力/天体/定居点 = 它的唯一名），不是整数编号；`--apply`
  diff 里的 `city`/`ship`/`faction_id` 写名字。
- **别让舰队维护费越过生产**：造舰预算会被「维护费 ×4 预留」封顶，但**流水的产出 vs 流水的
  维护/治理**才是生死线——`--control-plan <faction>` 先看 `verdict`，`bleeding`（净流为负）时
  先扩产、再扩军。否则帝国会被造船潮拖垮（几十回合内从霸权塌成 1 城）。
- **别当永久单极**：某势力实力占比超阈值 → 全网合纵 + 经济制裁 + 治理成本放大，过度扩张必被
  「均势」拉回来。想世界健康就做「强而不独」，不是「称霸到底」。

### 读世界在哪读、控制面在哪写

- **读（观察）**：`--index` 投影 + `planet_xq`（lean 主流 + 按 id 索引的 lazy 表：
  `ships`/`cities`/`factions`/`bodies`/`settlements`）。`factions` 表直接给每势力的
  `relations`（外交）/`resources`（库存）/自有城与舰；`ships` 给 effective 面板
  （attack/range/speed/upkeep/components）。要看 `State` 全量（relations 之外的原始结构）可
  `planet_x --round 0`（或 `--start ckpt --round 0`）拿单行完整 JSON。
- **写（控制）**：`planet_x --control` 拿可编辑模板，改进 `--apply` diff；`--control-plan
  <faction>` 先算「成本→收益」；`--control-schema` 查 diff 能写哪些字段。
