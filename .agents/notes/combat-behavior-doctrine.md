# 战斗系统 per-舰 行为风格 + 威慑 + 逐武器索敌（spec「控制属性.舰船.行为风格」）

> 状态 `[x]`（branch `feature/combat-overhaul`） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §21

> 用户裁决：每条风格轴取 `[-1,1]`、`0`=基线；**火力分配是武器自身可控属性（非舰船风格）**；**attack 是 per-武器**（每件武器是独立索敌单位）。目标选择所有自动逻辑通用一个基本权重：**距离 + 克制 + per-武器确定性随机扰动**；火力分配是叠在基本权重之上的一个层（本舰攻击历史新鲜度 × 武器 `fire_spread`）。

- **`ShipDoctrine{temper,lone_wolf}`**（per-舰，`ShipSpec.default_doctrine` 类默认、`Ship.doctrine` 实例值、`Ship.attack_hist` 火力分配记忆）。每条轴 `[-1,1]`，`0`=基线。〈警惕↔激进/风筝↔贴脸〉已**从行为风格降级**为普通舰船控制属性 `Ship.kiting`（见下），不再是风格轴。
- **`Ship.kiting`（风筝<->贴脸，普通舰船控制属性而非风格轴）**：`[-1,1]`、`0`=基线。**软目标**——Move/Follow/Dock/Idle 皆为软目标：附近有敌舰时此姿态**自动**移动本舰（对玩家 AI 一视同仁，玩家也不能硬控制）。`effective_retreat_hull` 收缩（风筝更早撤、贴脸更晚撤）；`kiting_dest` 做软移动（风筝钉在最远武器射程、敌近则拉开；贴脸压近到 `min_engage_range`）。`ShipSpec.default_kiting` 类默认；`web` 暴露 `ship_kiting`（读/写、钳 `[-1,1]`）。
  - `temper`（理智↔热血/欺软怕硬↔飞蛾扑火）：按**威慑对比**挑目标（用 `ln((my_det+1)/(tg_det+1))` 的 log-ratio，避免 `(my-tg)/(my+tg)` 在 my≫tg 时失去区分度）。
  - `lone_wolf`（护航↔独狼）：空闲舰是否护卫旗舰（`lone_wolf<0` 结伴护航，`>0` 独自就近接战）。
- **威慑 `sim::deterrence(ship)`**：综合战力（`attack×4+hull_max+shield_max×0.8+hardness×3+intercept×2`）+ `deterrence_radius`（默认 8 AU）内同势力友舰叠加。**注意**：同一簇里各舰的威慑相同（叠加成簇总量），所以 temper 区分的是「不同簇/整体强弱」而非簇内单舰——测试需把两目标放到**不同簇**才见分晓。
- **逐武器独立索敌**：`ComponentSpec.fire_rate`（发/时间，默认 1）+ `Weapon.fire_rate/fire_spread/seed`；`sim::fire` 改为**逐发 plan**（每发独立挑目标、按目标聚合伤害后再发事件/调关系，避免逐发关系掉太快）；`sim::fire_concentrate` 供集中火力便捷入口。`fire_spread`：`>0` 雨露均沾（越近打过的权重越低）、`<0` 死磕补刀、`0` 无偏置（基线）。
- **统一基本权重**（`autocontrol`）：`W_DIST×dist_score + W_CTR×weapon_counter + per-weapon noise`。`weapon_counter` 是**单件武器**克制（导弹 vs 点防、动能 vs 盾）。`weapon_noise` 由（武器 seed, 目标名）哈希、±0.06，确定性。
- **`web.rs`**：`ShipDoctrineEntry`（读）+ `ShipDoctrinePatch`（写，轴钳到 `[-1,1]`、只改本势力舰），`--control`/`--apply`/`--control-schema` 均含 `ship_doctrine`。
- **验证**：`cargo build --all-targets` 无警告；`cargo test` 44 lib（含 `spread_weapon_distributes_fire_across_targets`、`temper_biases_toward_weaker_or_stronger_deterrence`、`apply_ship_doctrine_patch`）+ 5 黑盒长局（`same_seed_reproduces_identically` 等）全绿；`--seed 42 --traj 8` 世界照常推进（有交战胜负、舰出厂/击毁）。`config/game.ron` 武器带 `fire_rate:1.0, fire_spread:0.0`（基线不变）。
- `[ ]`（可选）**把 doctrine 扩展到经济/造舰**（护航↔独狼、保守↔扩张等轴涉及舰队编成/资源投入），当前只影响战斗决策。
- `[x]`（branch `feature/behavior-redesign`）**行为枚举重定义**（`spec.md`「控制属性 · 舰船 · 行为（枚举）」）：攻击/轰炸都不需要行为，射程内自动发生。`ShipBehavior` 收敛为 `Move/Follow/DockCity/Dock/Colonize/Idle`（移除 `TargetShip{attack}`、`TargetSettlement{bombard}`；新增 `Follow(跟随舰船)`、`DockCity(停泊城市)`）。`Follow` 纯护航/追袭（所随舰**可是友方也可是敌方**），不拦截、不开火——攻击由统一基本权重自动接战完成；`DockCity` 驶向某城，敌对城在围城射程内自动轰炸。玩家路径与 AI 路径统一走 `autocontrol::auto_combat`（射程内自动开火/轰炸）。注意：kiting 是**软移动**（见上），连 Idle 舰在敌近时也会自动软移动，玩家不能硬控制。
- `[ ]` **政治系统：议题—立场—关切度 + 三因素关系模型**（完整设计见
  `political-system.md`；核心：关系 = 历史(静态) + 思潮(可变) + 利益(实时) + 均势；
  以「世界级议题」给权力关系提供目的，MOND 为第一实例。用户已裁决 4 处设计点：
  ①通用议题框架（MOND 首例）②分离「位置分歧 vs 零和竞逐」③历史静态 / 思潮可变 / 利益实时
  ④基座+记忆都要。M1=议题框架+基座+动机分解；M2=思潮漂移+大战略/政策；M3=零和竞逐+MOND 相位。）
- `[x]` **思潮最小功能实验（`Ideology`）**（branch `feature/ideology`，`src/model/faction.rs` 的
  `Ideology` + `Faction.ideology`，`src/model/game_config.rs::IdeologyConfig`，`src/sim.rs::step_ideology`，
  `src/world.rs` 播种、`src/projection.rs` 暴露）：4 条思潮轴（和平↔军国 / 科学↔技术 / 人民↔精英 /
  自然↔殖民，各 `[-1,1]`、`0`=均衡）逐回合按 spec 的「变化因素」向信号 target 靠拢并钳 `[-1,1]`：
  ①战争得失→军国/和平（敌舰被击毁+夷平敌城 − 我舰被击毁−我的城损失） ②飞船在 MOND 区(→科学) vs
  开采 MOND 区资源(→技术) ③经济净流(产出−维护−治理)→精英/人民 ④人均面积(总定居点面积/人口)→
  自然/殖民（`area_ref=0.15` 分出「拥挤大帝国→殖民 / 边地小势力→自然」）。确定性、无 RNG；agent 视图
  （`--round`/`--traj` 复用权威 `Faction`）与 `--index` 投影均暴露。验证：`cargo test --lib` 50 passed
  （+2 思潮守卫）、`cargo test --test longhorizon` 6 passed/5 ignored、`cargo build --workspace` 绿（lib+web）。
  seed 7 @ r30 各势力思潮收敛到可辨识画像（联合国=和平+科学+自然、欧盟=很精英、中国/美国=军国+殖民、
  科学组织=科学+自然、教团=人民+反殖民）。**目前思潮只是可读状态（尚无机械后果）**——下一步把轴线
  接进军事/科技/治理/经济修正（见政治系统设计 M2/M3）。

- `[x]` **WebUI 右侧「状态」面板：普通 state 全量读数（generic widget）**（branch `feature/web-state-panel`，
  `src/json.rs` + `web/src/lib.rs` 的 `InfoRoot`/`info_roots` + `web/static/jsonview.js` + `web/static/app.js::renderInfo`）：
  WebUI 以前只有左侧「可控 state」（控制面树，硬编码字段知识），普通 state（实体全量字段/事件/编年史/派生/配置）
  到不了前端。现在 `GET /api/state` 多带一个 `info: [{name, value}]`——**每个根都是模型的整份 JSON dump、零手工投影**：
  `state`（规范世界，含 control/scope）、`pre`/`post`（上一回合的派生态；`post.flow` 是步进函数**实际用过**的
  流量：每城/每势力产出、舰队维护费、治理成本/覆盖率，与 CLI `--save` 的 `RoundState` 同源）、`config`
  （game.ron 的全部调参表）、`session`（RNG 位置）。前端由 `jsonview.js` 渲染，它是 **schema-agnostic widget**：
  **不认识任何字段名**，只认 JSON 形状——object 全标量→键值行；object 全对象且 ≥3 项→映射表（首列=key）；
  array 全对象→自动表格（列=各元素键的并集，按首次出现顺序）；array 全标量→chips；其余→递归可折叠节点。
  于是**state 结构怎么改都行**（取证：临时给 `State` 加 `probe_new_field: BTreeMap<(String,u32), Vec<f64>>`
  后重建，UI 自动多出该字段、元组键显示为 `地球|7`，前端零改动；探针已回退）。
  配套：`src/json.rs` 是**结构无关**的 JSON dump 适配器（`to_value`）——RON 允许非字符串 map key
  （`invest_weights` 的 `(城市, 建筑id)` 元组键），直接 `serde_json::to_value(&state)` 会报 `key must be a
  string`，故由 `KeyAsString` 把任意键字符串化（元组→`"城|7"`、数字→`"3"`），保证整份模型都能落 JSON。
  UI：右侧边缘 panel（与左侧对称，只读），带 root tab、子串过滤（命中列名则只留该列；命中节点名则整棵子树
  照常展开）、全展开/全收起（展开状态按路径跨渲染保留）、点叶子复制 JSON 路径（`state.cities[3].loyalty`）。
  验证：`cargo test --workspace` 全绿（56 lib + 6 长局 + 2 web）；:3011 实机验证（自动表格/过滤/chips/
  复制路径/推进 3 回合后 flow 非空/左右面板+地图共存无回归）。
- `[x]` **把通用 widget 推广到其它读面（读面全面 generic 化）**（branch `feature/web-readside`）：左侧控制面树仍是手写 `KIND` 注册表（舰/预算/权重/建筑各有专用
  编辑器）——那是「写面」需要语义，暂不动；但**读面**（底部 readout、势力一览、舰面板）都可改成同一个
  widget 渲染 `world.info` 的子树，省掉一批手工投影。另：`StateView` 里给地图用的拍平字段
  （bodies/cities/ships/factions）与 `MetaView` 在 `config` 根出现后已属冗余，可让前端直接从 `info` 取，
  进一步删掉后端的手工字段。

  **已实现**（branch `feature/web-readside`）：①`StateView` 只剩 `{control, scope, info}`——删掉
  `bodies`/`cities`/`ships`/`FactionView` 与整个 `/api/meta`（`MetaView`/`BuildingMeta`）：那些都是
  手工挑字段的投影（会漂移、会漏字段）。前端改从 `info` 的 `state`/`config` 根取，显示名走
  `cfg.resources/structures/buildings/ships`（与 config 表同构）。有测试守住「StateView 不许再加
  给前端用的拍平字段」。②底部读面（原手写一行「势力资源/交战」）换成**选中对象读面**：地图点
  天体/城/舰、左侧点势力 → 在 `state` 根里按名字定位该对象（通用：扫顶层数组找 `name` 相等的元素，
  `KIND_ARRAY` 只是「这类对象住哪个数组」的最小提示），再用**同一个 widget** 渲染它的**整份记录**
  ——舰的 hull/shield/components/component_hp/doctrine/attack_hist/kiting…、城的 buildings 表、
  势力的 ideology/relations/resources… 全部自动出现，读面里不再有一行读字段的代码。选中即展开底部
  边缘 bar。③`renderDiff` 也改成结构无关（比较两帧 state 根里各数组的长度：`chronicle 0→3  events
  0→9  ships 21→13`）。④地图输入在 app.js 里从 state 根适配（只补一个 `id = name` 别名；
  **map3d.js 一字未动**，因为那份文件当时正被用户大改，避免撞车）。验证：`cargo test --workspace`
  全绿（56+6+2）；:3012 实机点城/舰/天体/势力→读面自动展开且内容正确、无 JS 报错、左树（含建筑编辑器
  与「+ 新建」）/右面板/推进/重建/应用均无回归。
- `[x]` **（事实澄清，非待办）首都是「控制 state 的一部分」，不是要补的东西**：`Faction` 刻意不存首都
  （`world.rs` 的 `initial_capital` 注释：单源无 shadow 双状态），**建世界时就把
  `control[势力].capital = Control::inherit(initial_capital(势力))` 播种好了**（联合国→月球、美国→火星、
  中国→地球、…，round 0 实测 9/9 势力都有该叶子），之后由 sim 的迁都步骤维护。所以：
  ①`info` 的 `state` 根里本来就能读到有效首都（`state.control.<势力>.capital.value`）；
  ②本轮删掉的 `FactionView.capital_body` 只是这个事实的**派生副本**（web crate 的 wire struct，
  由 `State::capital_body()` 算出来），删掉它是对的——但下游（map3d 的「首都色点」）必须改从控制叶子读，
  这一点最初漏了（见下条）；③绝不要在适配层另造兜底（我一开始写了 `'地球'` 兜底，属于凭错误前提
  引入的假事实，已删）。
- `[ ]`（下一步）**左侧控制面树的读侧也可以吃 `info`**：树节点现在按 `st.bodies/cities/ships/factions`
  自己查实体名（`KIND_ARRAY` 那套），可以进一步走「按路径取子树」的统一入口；另外 `behaviorSummary`
  仍是手写的行为→中文摘要（写面需要语义，暂可接受），若要彻底 generic，可让后端在 `info` 的
  `state.control.<势力>.ship_orders` 里就带上人可读摘要（那是模型字段，不是前端投影）。
