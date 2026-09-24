# 一切皆参数：删掉"画布"，相机归场景

> 2026-09-27（`feature/nurbs-op`）。用户在架构评审里连着给了三条裁定，逐句是：
>
> 1. **"不允许添加画布这个概念，一切皆参数"**；
> 2. **"camera 也不属于 graph 啊，camera 就是 scene scripts 中的一种普通数据"**；
> 3. **"为什么 payload 在 protocol?"** → 拆：线格式留 `px_protocol`，烘图那一半搬 `px_graph_schema`。
>
> 外加两条口径：**"投影也变成场算子的参数"**（一并进 params）、**"体积的 `res` 改绝对参数"**、
> **"分三步做" 被否，选"一次改完"**。这一篇记结果与代价。

## §1 删掉的四样东西（以及它们从前干什么）

| 删掉的 | 从前干什么 | 现在谁回答 |
| --- | --- | --- |
| `px_graph_schema::Grid` | 图交给算子的"画布"：`width × height + projection` | **参数**：`px_field_schema::params::Shape` |
| `Cache::grid()` | 画布的唯一出口（键里那份与算子手里那份必然同值） | 没有这个接缝了 |
| `Build::RESOLUTION_IS_CANVAS` | 每域手写"我的尺寸算不算画布"⇒ 决定要不要进键 | **参数表**：参数里有 `shape` 的自然换键，没有的自然不受影响 |
| `GraphSpec.{width,height,projection}` | 造画布 | 删。`GraphSpec` 只剩 `name` |
| `GraphSpec.cameras` / `Build::WITH_CAMERAS` / `Cache::cameras()` / `key_with_cameras` / `AssetManifest.cameras` / `PayloadBundle::to_bytes(id, cameras)` | 评审相机表随产物走 ⇒ 进键 | **场景脚本的数据**（`px-scene` 的 recipe：`cameras` 一栏） |

## §2 形状参数落在哪一档

* **生成类**（`field.constant` / `fbm` / `fbm3` / `ridged` / `ridged3`）：参数里多一栏
  `shape: Shape { width, height, projection }` —— 它们没有上游，尺寸只有这一个来源。
  `Shape` 手写了一条 `HashField`（`PxParams` 生成的是 `PxKeyed`，嵌套一层要自己接），
  口径与 derive 一样：**字段名也进键**。
* **过滤类**（`gradient` / `mix` / `warp` / `warp3` / `remap` / `craters` / `stamps`）：
  **不收形状**，输出**与上游同形**（`Field::like(value)`）—— 从前"画布必须与输入那张场同形"
  是调用点的纪律，现在**结构上只有一个来源**。
* **投影**也进参数（用户的裁定）。它从前按"只影响字节布局"**不进键**，现在进 —— 一并
  修掉了 `keys.rs` 与 `docs/system/assets.md` 之间那句自相矛盾的话。
* **体积的 `res` 从"占画布宽度的比例"改成绝对数**：`DensityParams::res_ratio` → `res`。
  ⚠ 代价正是从前那条注释担心的（"两边差一个绝对数就永远对不上"）：现在由**脚本**用
  **同一个值**喂场与体积（`nebula.rs` 里 `volume_shape` / `field_shape` 从一个 `shape` 变量推），
  "两处必须一致"这件事从此**看得见**。
* **网格**（`mesh.cubesphere`）的尺寸从**上游场**推（`cube_face_size(field.width)`）。

## §3 payload 那一刀

`Build` 这个 trait 从前住在 `px_protocol::payload`，于是"删一个参数"这种**契约**改动要去动
线格式 crate；而且它身上挂着两样纯烘图的概念（`WITH_CAMERAS`、`decode(…, node)`）。
现在：

```text
px_protocol::payload   PayloadBundle（CAS 里那份文件的形状：清单帧 + blobs）+ 清单指纹的 FNV
px_graph_schema::build Build（detail / encode / decode）+ 四个协议载荷类型的实现
各域 schema            自己那份实现（impl px_graph_schema::Build for Field / StarField / Curve …）
```

⚠ 一个**反例**说明"拆干净"不等于"全搬走"：`payload_fingerprint`（FNV）留在 `px_protocol`
——它算出来的值是**写进清单帧的字节**，而 `PayloadBundle::to_bytes` 要用它 ⇒ 搬走会把依赖
方向反过来。

## §4 相机为什么能整条删掉

关键事实（先查过再动手）：**渲染侧一个读产物相机的读者都没有**。
`px_render` 走的是 `SceneSpec.cameras`（场景文档自己那一栏），而 `px-scene::recipe`
本来就有 `cameras` 一栏（`Some("review") | None => …::review()`）。所以：

* 产物不再自带"该怎么看"；
* `px_render` 的 review 流程一个字不用改；
* **老 `.pxart` 仍读得进来**：`AssetManifest` 没开 `deny_unknown_fields`，多出来的 `cameras`
  键 serde 直接忽略（`#[serde(default)]` 也还在原来的位置上）。
* `px_graph::cameras::review()` 那张 12 视角表**暂时留在原地**（`px-scene` 仍引用它）——
  下一步把它搬到 `px-scene`（数据属于场景侧），那一步只搬文件、不改行为。

## §5 键这一侧净变化

```text
旧： op_id ‖ 接口哈希 ‖ 实现指纹 ‖ [该域的画布] ‖ 参数 ‖ 上游
新： op_id ‖ 接口哈希 ‖ 实现指纹 ‖ 参数 ‖ 上游
```

* 场：尺寸/投影**在参数里** ⇒ 改尺寸/投影**照旧换键**（§25.2 那类缺陷不可能复发）。
* 体积/网格/贴图/NURBS：参数里没有画布那一份 ⇒ 改**别的节点**的尺寸不会连带重烘它们
  （从前靠 `RESOLUTION_IS_CANVAS = false` 手写保证）。
* 相机：**不再进键**（它不在产物里了）。`ArtBundle` 的语义没变：`fingerprint` 只覆盖数据块。
* ⇒ **全仓键都变了**（参数结构改了）。老产物不会被误命中，只是要重烘；`.pxart` 字节格式
  本身没动（少了一个可选字段）。

## §6 诚实记账

* **N 份形状可以互相矛盾**：脚本里那一个 `shape` 变量只保证"同一张图里一眼看得见"，
  编译器不保证两个生成节点用了同一个值。过滤类算子的输入形状不一致会**当场炸**
  （`VolumeShape::matches` / 逐格读取的越界），不会静默出错位。
* **`GraphSpec` 现在只剩 `name`**：它还是"这张图的身份"（产物目录、清单名）。没顺手把它
  并进 `begin(name)` —— 那是一次纯改名的机械改动，与本轮无关。
* **判据的语义变了两处**（都改成了等价的新判据，不是放宽）：
  `the_canvas_size_is_part_of_the_key`（键里那一轴没了 ⇒ 改成"改生成类算子的形状参数必换键"）、
  `only_the_field_domain_bakes_the_canvas_into_its_size`（⇒ 改成"只有产出场的生成类算子的
  参数里有 `shape`"）。
