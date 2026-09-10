# 用 Python 统计地编辑控制面 diff（`play/planet_x_ctl`）

> 状态 `[~]`（spec 已定、kit 开工） ｜ 索引：[notes.md](../notes.md) ｜ 关联：
> **`engine-data-plane.md`（先读那篇：引擎产出/接受什么，本 note 是它的客户端篇）**、
> `lazy-index-pandas.md`（读侧那套 Python kit `play/planet_xq`）、`control-live-layers.md`
> （写面的形状：三态 + 链 + 写值即接管）、`name-as-unique-key.md`（名字即唯一键）、
> `agent-control-long-game.md` §1/§7/§8（预算封顶、幽灵权重、只报丢弃不报生效）

## 0. 目标

agent 玩长局时，「施政」这一步现在靠**手写 JSON diff**（`--apply steer.json`）。这在长局里
退化得很快：城市几十座、舰十几艘、名字还会换代，凡是「按统计量批量决定」的意图
（按维护费占比压预算、按忠诚/距离撒娱乐预算、按舰级设默认指令）都得靠人肉枚举。

要的是：**读侧用 pandas 筛、写侧产出合法 diff** —— `play/planet_x_ctl`，与 `play/planet_xq`
并列的第二个 uv 工程。**引擎不为此加任何功能**（`engine-data-plane.md` §1）：

* 引擎管**还不存在的实体**的规则（链上的默认、舰船模板）；
* kit 管**现存**实体的批量与统计操作，**展开成显式叶**。

### 0.1 这不是假想的痛点（`play/exp2` 的现场）

| 证据 | 说明 |
| --- | --- |
| `diff_p1..p7.json` | 一次实验里**手写了 7 份**施政 diff |
| `apply_p3.jsonl` | 那一次的回执：**19 片叶丢了 6 片**，原因清一色「`创神星采矿站` 属于 **欧盟**，不是 中国 的城」——手抄城名时没核对归属 |
| `control_r144.json` | 控制面模板已 **86 KB**（r12 时才 23 KB） |
| `meta_ships.py` / `brief.py` / `timeline.py` | 一局里**临时写了 4 个一次性脚本**，做完就扔——正是本套件该收编的 |

手抄会错在四个地方（都是形状本身的坑）：**名字换代**（`ShipId = String`；p192 里已有
`方舟3/4/5`、`摆渡7`）· **`(city, building)` 的 building 是城内下标**（`src/model/control.rs:47`）
· **`mode` 省略 = 写值即接管** · **`deny_unknown_fields`**（打错字段名当场报错——好事，但手写正是错字来源）。

## 1. kit 的职责与 API

```python
import planet_x_ctl as ctl

s = ctl.surface("ckpt_r12.ron")            # 控制面（= `--control` 的读面，读面即写面）
s.factions                                  # 势力名列表
s.faction("中国")                            # 该势力的叶片：ship_orders / default_role /
                                            # 预算 / 权重 / loyalty_budget / buildings / capital
s.leaf("中国", "ship_orders", "长城")        # 单叶：值 + mode（三态）

ships  = ctl.ships("ckpt_r12.ron")          # 投影的 ships 表（名字 join 控制面）
cities = ctl.ships_and_cities("ckpt_r12.ron")   # 便捷 join：舰/城 × 其控制叶 × 归属

mine = ships.query("faction_id == '中国' and hull > 0")

# —— 通配：引擎没有通配，这里展开成显式叶（同一个东西，只是不用手抄）——
s.set_mode(mine, "Auto")                    # 全舰队交回系统（`{ship, mode:"Auto"}` 逐条）
s.set_mode(mine, "Player")                   # 全舰队归我
s.set_default_role("中国", "Freight", mode="Player")   # 一片叶：全舰队转运输（**长期倾向**才有舰队级默认）
s.set_kiting(mine.query("class == 'battleship'"), -1.0)                # 贴脸
s.set_budget("中国", "construction_budget", {"铁": 4.0, "硅": 2.5})     # 值 + 隐含接管

diff = s.emit()                             # → {control:[...], scope:{...}}，可直接 --apply
ctl.write(diff, "steer.json")
ctl.verify("ckpt_r12.ron", "steer.json")    # 只读试算：前后读面 + 回执（见 §1.2）
```

1. **`roster()` —— 编制表（本套件的核心，不是附注）**：名字是唯一键，但**名字会换代**
   （上一艘沉了才换代——那正是手抄最不可靠的时刻）。所以要有「`第1舰队·旗舰 → 方舟3`」这层：

   ```python
   spec = [("第1舰队·旗舰", "class=='cruiser' and faction_id=='中国'"),
           ("第1舰队·护卫", "class=='corvette' and faction_id=='中国'")]
   r = ctl.roster("ckpt_r12.ron", spec)     # → DataFrame[faction_id, slot, ship_id, class, hull, matched_by]
   ```

   **刷新规则必须写在配方里**（"旗舰 = hull_max 最大的巡洋舰，同分取最老的"），不能留在
   agent 脑子里——否则断一回合就永远补不回来。边界：编制表是**意图**不是**持续承诺**；
   "旗舰沉了自动补一艘"该由引擎规则或"配方每回合重跑"承担。

2. **`verify(ckpt, diff)` —— 客户端补丁，补 §8「只报丢弃不报生效」**：
   `--apply` 是**先叠加、再输出**且**不写盘**（`src/main.rs:250` 在 `--control` 的 `:304` 之前，
   写盘只由 `--save` 触发），所以验证是**纯只读**的：

   ```
   planet_x --start ckpt.ron --apply my.json --control   # stdout=叠加后的读面, stderr=回执
   planet_x --start ckpt.ron --control                    # stdout=叠加前的读面
   ```

   一比就知道：想改的叶变了没、**顺手接管了别的叶没**（`NOTE_APPLY_TOOKOVER`）、被丢弃了没
   （`WARN_APPLY_SKIPPED`）。**但它只能证明"引擎收下了"，不能证明"局面按你想的走"**，别自欺。

3. **同回合变换（硬约束）**：读 ckpt → 出 diff → 应用到**同一个** ckpt。`building` 是城内下标，
   只有同回合才自洽；跨回合拼凑必错。

### 1.1 读面的两半怎么拿到

* **控制面**：`--control`（读面即写面）。若 `engine-data-plane.md` §2 的 tidy 表落地，则改为直接
  join 投影（`control` 段 + 每实体 `effective`），**不要**自己重实现 `resolve_chain`——那是漂移源；
  `effective` 列由引擎给。
* **世界事实**：复用 `play/planet_xq` 的懒表读取（`import planet_xq`，或抽公共 `read_index`），
  **不要**复制 join 逻辑。

### 1.2 一个必须写进 README 的取值坑

「交回上层」（`mode:"Inherit"`）**只在舰队默认是 `Player` 时才是干净的**：`ship_behavior`
（`src/model/state.rs:186-193`）在"叶 Inherit + 舰队默认非 Player"时**回落到叶上那个可能已过期的
记录值**。归属是 `Auto` 时自愈（系统下回合重写）；归属是 `Player` 时会长期显示旧值。
处理：凡"释放到上层"，**同时**把该轴的舰队默认写成 `Player`（一片叶），语义闭合。
⚠ **指令例外**：它没有舰队默认叶（2026-10 删除，见 `blueprint-stance.md`），释放只是"叶里那句旧值继续算数"。

### 1.3 边界（别让 kit 变成"每回合重跑的假公式"）

kit 只能产出**一次性数值**。「跟着产出走」「维护费不超过产出的 X%」这类**持续意图**必须由引擎
表达（`BudgetPatch.value` 支持 `{"frac_of_production":0.3}`、`upkeep_ceiling`），kit 顶多在落地前
当"预览计算器"。两边各补一半，别互相假装。

## 2. 裁决记录（用户已确认）

* ✅ **一次性 diff vs 可复现配方 → 配方**：写成 `policy(surface, ships, cities, seed, round) -> diff`，
  可重放（同一 ckpt 重跑断言 byte-identical）、可进 git、可**先预览**（拿旧 ckpt 试跑"当时我会怎么写"）、
  战记能附上"凭什么这么算"。**编制表的刷新规则尤其必须在配方里**（§1.1）。
* ✅ **投影的 control 段 → 要**（但由**引擎**加 tidy 表，见 `engine-data-plane.md` §2；kit 不再
  shell out `--control` 做统计）。
* ✅ **套件位置 → `play/planet_x_ctl`**，与 `play/planet_xq` 并列的新 uv 工程，复用懒表读取。
  记得加 gitignore 例外（`/play/*` 默认忽略，`!/play/planet_xq` 是现成的先例，新目录要照抄一行）。
* ❌ **引擎侧通配/清叶动词 → 已否决**（理由与替代见 `engine-data-plane.md` §1.1）。

## 3. 落地记录（`[~]` 套件本体已建，在 `play/planet_x_ctl`）

1. `[x]` `play/planet_x_ctl/`：`pyproject.toml`（uv，唯一依赖 pandas）+ `planet_x_ctl/__init__.py`
   + `README.md`（双语：分工 / API / 同回合约束 / §1.2 的取值坑 / 引擎缺口）+ 自断言 `demo.py`
   （自己生成 fixture，无网络）。`.gitignore` 加了一行 `!/play/planet_x_ctl`。
   **实测**（`uv run python demo.py`，退出码 0）：批量 Auto/Player 各 5 叶、0 跳过 0 接管，
   值的字节级不变；统计封顶配方 29 叶全落地、`took_over` 恰好是刻意的那一片；
   两次跑配方逐字节一致；"写值不写 mode / 死舰名 / 换城 building 下标 / 未知资源"四个护栏
   在**配方期**就拒绝；畸形 diff → `exit 10` 且不抛异常。
2. `[x]` **确定性**：同一 ckpt + 同一配方 ⇒ 逐字节一致的 diff（demo 里断言）。
3. `[ ]` **`agent-play.md` 加一节「用 Python 写施政」**（引擎侧的原生写法已经写进 §3/§4 了）。
4. `[~]` 换后端 + 去掉本地重算：
   * `[x]` ~~自己算了一份 `effective_*_approx`~~ → **已解**（`feature/read-face-parity`，
     见 `control-live-layers.md` §13.3）：`ships()` 现在优先读引擎 ships 表的
     `order_effective_mode`/`order_effective`/`order_source`（**含设计图层**——本地那份没有它，
     对"按图造的舰"是错的），只有**旧 index 目录**才退回本地近似，且列名带 `_approx` +
     布尔列 `effective_order_from_engine` 标明来源。
   * `[ ]` `surface()` **仍然 shell out `--control`**，这是**有意保留**的：那面模板就是写面
     （读面即写面），`idx/control.jsonl` 发的是叶自己的值/表态、发不出模板形状（`null`-vs-缺席
     的 presence 语义、`remove` 的省略…）。要改的话先想清楚"模板从哪里来"。
5. `[ ]` 一个示例配方进 `play/exp*/recipes/*.py`（demo 里的统计策略已经很接近，可直接搬）。
6. `[x]` **跟上引擎的两片新叶**（本轮补，教训见 `engine-data-plane.md` §8.7）：
   `LEAF_KINDS` 补 `default_doctrine`/`default_kiting`（少了它们**不报错**，只是这两片叶
   从 `surface()` 里静静消失；demo 的叶计数 237 → 255 就是它们），加两个 setter，
   并让 `set_kiting`/`set_doctrine` 也要求显式归属（`mode=` 或 `take_over=True`）——
   引擎侧它们早就是三态叶了，kit 文档还停在 "engine gap"。
7. `[x]` **两轴叶的护栏**（`Surface._require_both_axes`）：`default_doctrine`/`ship_doctrine`
   一片叶装 `temper`+`lone_wolf`，而在叶**还不存在**时只写一条轴，引擎会把另一条初始化成
   `0.0`（不是保留出厂值）——`0.0` 是个正常取值，事后看不出来。kit 现在在配方期直接拒绝。
   ⚠ 引擎侧要不要自己种上缺的那条轴，见 `control-live-layers.md` §3.1（**待裁决**）。
   实测：读面读得到、写得到（`took_over` 恰好两片）、落地后有效值随势力默认走
   （中国 5 艘舰 `kiting` 全 = −0.6）；已存在的叶仍允许只改一条轴。

8. `[x]` **叶种类表不再手抄**（2026-10，`feature/control-tree-retire`）：`LEAF_KINDS` 从写死的
   dict 换成**懒加载的 `Mapping`**（`_LeafFacts`），事实来自引擎的 `--control-schema`
   （`src/control/leaves.rs` 的 `leaves`/`actions` 段）——顺手删掉
   `_VALUE_FIELD`/`_TWO_AXIS_KINDS`/`_COMPOSITE_KINDS`/`_BLUEPRINT_FIELDS` 四张表，
   以及没有任何调用方的 `_KEY_FIELDS`：
   * `_leaf_value` 按 manifest 的 `values` 取（一个字段直取、多个给 dict）⇒ **加一条轴不用改 kit**；
   * `_diff_fields` 的「被请求字段」= `{mode, remove} ∪ values(kind)` ⇒ 两轴不再需要特例分支；
   * `_leaf_fields`/`_diff_fields` 过滤身份键改成**按这片叶自己的 `keys`**（旧并集语义会把
     `invest_weights` 顺带带过来的 `resource` 属性一起抹掉）；
   * 「叶不存在」那一行把**全部值字段**给 `null`（旧代码只写一个 `value: null` ⇒
     两轴风格叶的另一条轴会在 `verify` 的 before/after 里凭空消失）。
   验证：`demo.py` **全部断言通过**（这一局 297 片叶）、数据级四组 67 条全绿；纪律
   （`leaves` ∪ `actions` ∪ `{faction_id}` ≡ `FactionControlPatch.properties`，双向）
   由 `play/tests/g4_spec.py` 守，不再靠"记得改这边"。
   这一条与 web 那半是**同一件事**：三端（引擎 / kit / WebUI）现在读同一份声明，
   见 [`web-control-spec.md`](web-control-spec.md)。

## 4. 复现 / 验证

```bash
cd play/planet_x_ctl && uv sync && uv run python demo.py     # 自断言，退出码 0 = 全过
# 引擎侧对照（同一份 diff 手写版长什么样）：
planet_x --start play/exp2/ckpt_r12.ron --apply steer.json --control 2>receipt.jsonl
# 读派生表（引擎算出来的量）：
planet_x --seed 7 --round 6 --index out/ && python -c "
import planet_xq; q = planet_xq.load('out')
print(q.derived('faction_process', 6)[['faction_id','upkeep']]); print(q.control(6)['kind'].value_counts())"
```
