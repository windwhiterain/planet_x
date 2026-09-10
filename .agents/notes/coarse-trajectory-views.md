# 超长轨迹的「粗粒度 / 降采样视图」

> 状态 `[~]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §15

> 问题：`--round 3000` 输出 3001 行全量 JSON，agent 上下文/管道撑不住。要让 agent 先看
> 「粗粒度」再决定是否放大。落了一套，剩下的是候选。

**已落地（`src/main.rs`）**：
- `[x]` **`--every K`（降采样快照）**：`--round N --every K` 只发 round 0、K、2K… 行（仍是
  **全量** agent 视图），超长轨迹秒变可读采样。`--traj N --every K` 同理降采样 `trajectory`
  数组。配 `--start`/`--save` 做「先粗看、再放大」的分段叙事。
- `[x]` **`--digest K`（语义编年史窗口 / 故事板）**：`--round N --digest K` 每 K 回合发**一行
  语义摘要**——该窗口各势力 `city_count/ship_count/fleet_value`、总 `power_share`/`hegemon`/
  `coalition_members`、`wars`（交战对）、`events`（各类型计数）、`story`（本窗口触发的剧情节拍
  id）。`--digest 100 --round 3000` → 30 行，就是「编年史的粗帧」。这是超长轨迹真正能
  读的故事板：agent 扫完决定放大哪些窗口，再 `--start ckpt --round <span>` 精读。
- smoke：`--round 50 --every 10` → 6 行（0,10,…,50）；`--round 48 --digest 12` → 4 行，
  win1 的 `story=[northern_watch,first_raze,kuiper_boom]`、`wars=(1,3)…(5,8)`、`events`
  有 attack/siege/revolt/…；`--traj 30 --every 5` → 7 帧。

**候选（未做）**：
- `[~]` **极致「画像」**：只发一条极简标量时序（每窗口：城数/舰数/世界价值/最强势力份额），
  几乎零上下文，适合先看走势。**已落地一个纯 jq 版**（`play/jq/view_profile.jq`，一并算出
  wars 数）：`jq -cs -f play/jq/view_profile.jq traj.jsonl` → 每回合一行
  `{round,cities,razed,ships,fleet_hull,top_name,top_share,wars}`。seed 7 240 回合：整条
  轨迹从 **8.7 MB 压到 ~29 KB**，一眼看清「22 城→13 城(9 夷平)→再殖民→尾声 17 城 7 舰队」
  的走势与 top_share 轮换（0.18→0.31→0.53(矿业)→0.5→…→0.53(中国)）。**Rust 侧**可把
  `top_share` 换成真实 `balance_picture.power_share`、并加 `--profile K` 窗口化（见 `semantic-view-api.md`）。
- `[ ]` **分层缩放 CLI**：`--zoom from to --every k`（在已存 checkpoint 上精读某窗口）——
  现在可手工 `--start ckpt --round <span> --every k` 达成，值得包装成一条命令。
- `[ ]` **窗口事件文案**：`--digest` 的 `events` 计数之外，附几条**一句话**事件摘要
  （如「第 210 回合：联军包围霸权，铂被封锁」）——半叙事粗帧。
- `[ ]` **jq 现成的粗聚合**（零代码，留给 agent）：`planet_x --round 3000 | jq -s '[group_by
  (.round/100|floor)[] | {from:.[0].round, ncity:(map(.cities|length)|add/length)}]'`。
