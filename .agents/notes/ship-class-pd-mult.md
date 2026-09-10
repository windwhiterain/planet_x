# 舰级数值按最新 spec 对齐（新增「点防御修正 pd_mult」舰级属性）

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §10

已落地：`.agents/spec.md` 的最新修订把每级舰重新定性（护卫舰=哨戒、驱逐舰=远洋部署、
巡洋舰=扛伤害、航空母舰=远程放风筝、战列舰=玻璃大炮），并**新增一个舰级属性
「点防御修正 pd_mult」**——舰级=平台修正器的一环（此前缺失该斜率，点防不随舰型变化）。
改动：
- `src/model.rs`：`ShipSpec` 新增 `pd_mult`（`#[serde(default=default_mult)]`，旧配置无它也
  能加载）；`ship_panel` 的点防拦截改为 `cs.intercept × base.pd_mult × eff`。
- `config/game.ron`：五级舰全部按 spec 重填——新增 `pd_mult`（护卫/战列 1.4 高、驱逐 1.0
  中、巡洋/航母 0.9 低），并重分布其余修正系数（护卫=加速度极高 1.8 + 点防高 + 速度中；
  驱逐=高速 1.3 + 高再生；巡洋=重甲重盾 + 盾再生高；航母=超远程 1.8 + 各项低；战列=攻
  极高 2.0 + 射程高 1.3 + 点防高，槽位降为 3）。战列 hull 26→30（中）。
- `src/sim.rs`：`choose_loadout` 的组件得分把 `intercept` 乘上本舰级 `pd_mult`（点防模块在
  pd_mult 高的舰上更值得装）。新增单测 `ship_panel_scales_intercept_by_class_pd_mult`。
- `src/agent.rs`：meta 的 `ships` 表暴露 `pd_mult`。`src/main.rs` help 文案同步。
- 验证：`cargo test --lib` 45 项全过；`cargo test --test longhorizon` 5 个长局守卫全过
  （多极/反僵尸/价值有界/确定性/无 NaN）；`diagnose_long_horizon`（3 种子 × 3000 回合）
  0 僵尸、~20-21 城、44-50 舰、无 NaN。
