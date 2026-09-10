# 《行星X》笔记索引

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
| `[~]` | [治理与忠诚度](notes/governance-loyalty.md) | 治理开销与忠诚按「距首都 × 人口超载」计费，付不起就离心；已实现城市倒戈换主。 | 「倒戈目标要治理得住」的过滤；忠诚影响生产未做 |

## 军事 · 科技 · 天文

| 状态 | 条目 | 一句话 | 剩余 |
| --- | --- | --- | --- |
| `[x]` | [战斗行为风格](notes/combat-behavior-doctrine.md) | per-舰 `doctrine`、逐武器索敌与统一权重已落地，思潮实验也接上。 | doctrine 扩到经济/造舰；政治系统 M1–M3 |
| `[ ]` | [时代与科技演进](notes/eras-technology.md) | 用解锁式舰级、材料升级与舰种分支，给上千回合铺时代节奏；三条都还只在纸面。 | 舰级解锁、结构演进、设计图分支全未开工 |
| `[~]` | [军事与战斗](notes/military-combat.md) | 拟真战斗与舰船定制（组件/护盾/点防/命中折减）已落地，AI 拟人化那批也做完。 | 舰船退役换装、换模块/再装配；长局可玩性 |
| `[ ]` | [舰船设计图](notes/ship-blueprint.md)（设计长文） | 非控制属性（面板/选装/造价）放在**建造单位**上作为出厂快照的设计图；也是「还不存在的实体的规则」的家（含按舰级默认）。 | 全部；文内 §3 四条语义已裁决（快照 / 三态 / `choose_loadout` 降级 / refit 出本轮） |
| `[~]` | [MOND 引力异常](notes/mond-anomaly.md) | 异常区导航偏移已实现（崇拜教免疫）；战斗光环、矿产红利与科技扩散未做。 | 异常区战斗光环；矿藏加成；MOND 扩散 |
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
| `[~]` | [控制属性 = 活层](notes/control-live-layers.md) | 三态归属 + 势力级默认指令 + 写值即接管 + **风格活层（doctrine/kiting）**全部落地；§4 四条已裁决；**web 侧风格两行 + 逐舰风格叶已实机点过**（见 §7）。 | `agent-play.md` 跟改；`projection` 的 control 表还缺四条风格叶 |
| `[~]` | [引擎=数据平面，Python kit=策略平面](notes/engine-data-plane.md) | 引擎产出 tidy 统计表 + 接受同形状 diff：`flow`/`city_flow`/`control`/`scope` 四表 + `--derived` 已落地，消费者（`planet_xq`）也已接上；通配/编制表全归 kit。 | "AI 掷了什么"要单独捕获（`pre` 不是它）；`--control` 的 2 位舍入；`spawned_round` |
| `[~]` | [Lazy 索引分析层](notes/lazy-index-pandas.md) | 重型字段拆成按 id 的懒表，Python/uv 套件读 schema 后 join 分析；**新增 `derived` 段与 `q.flow()/q.control()` 等派生表读法**。 | parquet、更多 lazy 字段、剩余统计函数 |
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
| `[~]` | [用 Python 统计地编辑控制面 diff](notes/python-control-authoring.md) | `planet_x_ctl` 已建（`play/` 并列 uv 工程）：批量改归属、编制表、统计配方、`verify` 全在 demo 里自断言通过。 | `agent-play.md` 一节；`surface()` 换 join 后端并删掉本地 `_approx` 重算 |
| `[x]` | [名字即唯一 key](notes/name-as-unique-key.md) | 实体身份统一用名字作主键，删掉数字 id 与影子结构。 | web 投影收敛；派生字段预计算 |
| `[ ]` | [Schema/查询架构调研](notes/schema-query-architecture.md)（设计长文） | 四套并行投影、隐式 schema、宽容查询、无版本迁移——诊断 + 分层方案。 | 文内 §6「触手可及的首步」 |
| `[ ]` | [Schema 查询重构（候选清单）](notes/schema-query-refactor.md) | 四痛点里的 P0–P3 已落地：`meta_value` 止血、schema 自描述、响亮失败、版本迁移。 | P4 收敛并行投影（拆 `AgentState` 镜像 struct） |
| `[ ]` | [语义视图 API](notes/semantic-view-api.md) | 把裸 jq 降为逃生舱；语义 view 工具只在 jq 侧做了 PoC（压缩约 34×）。 | Rust 侧 `view` 命令；工具层参数校验 |
| `[x]` | [定居点名字 key](notes/settlements-lazy-table.md) | `Settlement` 改按名字引用，投影新增懒表 `settlements`，测试全绿。 | — |
| `[x]` | [统一总结指标](notes/unified-metrics.md) | 总结指标由步进中间量聚合，agent 视图与 `--digest` 同源、不再重算。 | 治理中间量并入 metrics；Web 是否复用待定 |
| `[x]` | [权威 schema 贯彻](notes/wysiwyg-resource-keys.md) | 资源 key 统一成中文可读名、删掉镜像结构，视图直用权威类型。 | 派生字段要 agent 现场 jq 计算（或加语义视图） |

---

## 快速参考：验证手段

- 单元测试：`cargo test --lib`
- 长局快守卫：`cargo test --test longhorizon`
- 长局诊断（慢，可打印）：`cargo test --test longhorizon diagnose_long_horizon -- --ignored --nocapture`
- 一次简短观察：`cargo run --bin planet_x -- --seed 7 --round 30 --digest 10`（每 10 月一行故事板）
- 一键拿故事素材：`planet_x --seed 7 --round 60 --index out/`，再用 `play/planet_xq`
  (`planet_xq.load('out').facts`) 读主流与 `chronicle`（累计编年史按 `(round,id)` 去重）。
