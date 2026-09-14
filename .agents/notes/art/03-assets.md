# 产物与协议

一份 `.pxart` 里到底有什么、域（equirect / 八面体 / cube / cubemap）怎么定，以及怎么判断两份产物有没有差别。
（PCG 侧的缓存键与版本号在 `02-pcg.md`。）

## §23 四种域，各自的问题

极点那团「放射状条纹」是**两个问题叠在一起**：**① 几何退化**（UV 球的极点是一圈退化三角形，每列两个，法线与 UV 都塌到一点）—— 换网格能治；**② 域不匹配**（equirect 把整整一行纹素压到一个点上，极区方位角转一点点，采样的 x 就跑过整行）—— 只要域还是 equirect，换什么网格都还在。

- **equirect**（经纬度，一张 2:1 图，`θ = v·π`、`φ = u·TAU`）：退化映射 —— 极点一整行纹素覆盖整整一圈，放大时是一整行纹素的径向拉伸；极区纹素**冗余**；`ClampToEdge` 下 u 方向列 `W-1` 与列 `0` 是「夹住」不是「接上」（一个纹素宽，给 u 设 `Repeat` 修）；真正稀缺的是**赤道上的方位向**。
- **octahedral**（八面体摊开是一个正方形：正中菱形是 +Z 的四个象限，四个角是 −Z 的四个象限）：每个纹素对应一块确定的立体角，texel 密度变化 ≤ 2×；但**折叠线两侧的相邻纹素是镜像的不相邻方向** ⇒ 双线性/各向异性/mip 过滤跨过折叠线必混入无关纹素，棱上必然有线（布局固有性质）。
- **cube**（3×2 图集 + gutter，布局见 §31）：六个独立正方形面，无折叠 ⇒ 跨面边缘两侧纹素对应几乎相同的方向，过滤几乎正确，加 gutter 后完全正确；奇异点从两极搬到立方体的**十二棱与八角**。
- **cubemap**：六个正方形面按方向采样（天空盒的用法），不以网格 UV 为依据 ⇒ 没有极点问题。

**② 的正解是「按方向求值的 3D 噪声」，而且不需要改域格式**：`noise3(x, y, z)` 的 `(x, y, z)` 是**方向向量**，定义域是 R³、球面只是其中一张曲面 ⇒ 无坐标奇异点、极点连续、`φ=180°` 处无缝、特征尺寸处处相同。**伪的那种**是把极坐标 `(θ, φ)` 喂 2D 噪声 —— 只是 equirect 换了个参数名，极点奇异点原封不动。

**换网格只治 ①**：Bevy 的 icosphere（`SphereKind::Ico`，极轴 **±Y**，`inclination = acos(point.y)`）没有极点顶点，扇没了；但它**用的是同一套 equirect UV 公式** `[0.5 - atan2(z,x)/TAU, acos(y)/PI]` ⇒ 放射状山脊照样在。⚠️ 从 UV 球（极轴 **±Z**）换过去时必须一并删掉 `Quat::from_rotation_x(-FRAC_PI_2)`，否则极点转到侧面。

### §23.6 真球面噪声与备选构造

在球面上求值的 3D 噪声即「真」的球面构造；备选（未采用）：**球谐（SH）**（带宽受限、天然无缝，擅长低频，代价 O(波段²)）、**球面 Voronoi**（Fibonacci 球面均布种子 + 大圆距离 ⇒ 等尺寸胞元，做板块构造最合适）、**HEALPix / 测地网格**（等面积**域**，天文学的标准采样网格）。

### §23.7 `spherical` 默认开

`field.fbm` / `field.ridged` 各有 `spherical: bool`，**默认 `true`**（`px_ops/src/ops/{fbm,ridged}.rs`）：内部把 `(u,v)` 映成方向再求 3D 值噪声，**产物格式、网格、渲染器一行都没改**。极点干净（地形平滑翻过极点，只剩冰盖），特征尺寸处处一致；代价 **8 个晶格点 vs 2D 的 4 个 ⇒ 约 1.4× 耗时**（29 ms vs 18–22 ms）。对照图 `target/sphere3d-rocky.png`、`target/sphere3d-desert.png`。

⚠️ **频率语义变了**：单位球绕赤道一圈是 2π，`frequency = f` ⇒ 赤道上约 `2πf` 个特征，旧参数（3.2 / 11.0）按 3D 语义相当于「赤道上 20 个格子」，必须重调；现为大陆 `0.55`、山脉 `1.9`、沙漠台地 `0.45`、峡谷 `3.2`。`field.warp` 仍是**图空间**扭曲（在像素坐标里偏移采样、极点附近并不球面正确），后由方向空间的矢量位移取代（`04-geometry.md` §33）。

### §23.9 极点的问题是冗余，不是分辨率不足

方向向量在球面上均匀、3D 噪声的晶格在 R³ 里均匀 ⇒ **噪声本身在极点没有任何分辨率问题**；不均匀的是**烘焙网格**（equirect 把 v 均匀分行、u 均匀分列）：

- 纬度 0° / 60° / 80° / 89° 的方位向弧长 / 纹素：`2π/W`（1×）/ `2π·0.500/W`（2× 密）/ `2π·0.174/W`（**5.8× 密**）/ `2π·0.017/W`（**57× 密**）。

（W=384、H=192 时：赤道上方位向 `2π/384 = 0.01636`、经向 `π/192 = 0.01636` —— **赤道上正好是方纹素**，这就是 2:1 贴图配 360°×180° 的意义。）⇒ **equirect 稀缺方向是赤道方位向，极点是把纹素浪费掉了**（算力/显存，不是画面）。剩下真正会咬人的两点：**放大时极点的各向异性**（纹理足迹纬度方向极窄、方位方向极宽 ⇒ mip 要么糊要么闪，要放大到极点才看得见）、**±180° 的采样方式**（修法：给 u 轴设 `Repeat`，约 3 行）。cube 域的好处不是「修正错误」，而是**提高纹理预算效率**（同一面内最差/最好只差约 1.7×）。

## §25.1 球面贴图的约定与过滤

- **UV 约定统一为 `v = y/(h-1)`，第 0 行 = 北极**：`Field::uv`、`noise::direction`（v=0 ⇒ θ=0 ⇒ 北极）、网格 UV 三处一致。
- **±180° 接缝**：表面纹理 u 轴 `Repeat`、v 轴 `ClampToEdge`；环纹理反过来。
- ⚠️ 一旦给 `Image` 显式设 `ImageSampler::Descriptor`，就**不再继承 `ImagePlugin` 的 `default_linear()`**，而是取 `ImageSamplerDescriptor::default()` —— 而 `ImageFilterMode::default()` 是 **`Nearest`**，画面立刻变方块；必须显式写 `mag/min/mipmap_filter: Linear`。
- **值噪声的值长在轴对齐的立方晶格上**，高频八度会把方格暴露出来（`frequency 0.55 × 2⁵ = 17.6` ⇒ 晶格间距 `1/17.6` 单位 ≈ 球面 3.2° ≈ 屏幕 9 px）⇒ 换成 **Perlin 梯度噪声**（12 个立方体边梯度、8 角点、smoothstep 权重），`field.fbm` / `field.ridged` 升到 **v3**。

### §25.2 画布尺寸必须进键

`GraphSpec.width/height` 影响每个节点的输出，一度却没进缓存键：把画布从 384×192 改成 768×384 后**所有节点照样命中**，产物还是 295111 字节的旧尺寸 —— 是 §19.1 那条 ⚠️ 告警（「图 planet 的源码变了但 GRAPH_VERSION 仍是 1」）把它喊出来的。已修：`node_key` 增加 `canvas: (u32, u32)`，并补测试 `the_canvas_size_is_part_of_the_key`。

### §25.3 代价（768×384 + Perlin）

单个噪声节点烘焙 18–29 ms → **287–297 ms**；planet 图全量 67 → **619 ms**；desert 图全量 96 → **779 ms**；出图 0.34 → 0.37 s；**整轮热路径 0.49 → ≈ 1.0 s**。若嫌慢，可把画布降到 512×256（约省 55%）。mip 见 `04-geometry.md` §26。

## §28 八面体域：极点退化是滤波治不了的

**为什么必须离开 equirect**：经纬度图的极点纹素**覆盖整整一圈**，是**退化映射**，任何滤波都只能「平均掩盖」；八面体图**每个纹素对应一块确定的立体角**，texel 密度变化 ≤ 2×（cubemap 单面约 1.7×，同一量级）。

形状：`px_protocol::art` 提供 `octahedral_direction` / `octahedral_direction_y_up`（**双方共用**，它是产物格式的一部分）；`GraphSpec.projection` **和画布尺寸一样进缓存键**。三条仍然成立的接线事实（八面体映射还在代码里，作为「为什么最后不用它」的证据链）：

- 烘焙与网格必须用同一族映射：`px_ops` 用「极轴 +Z」、网格用 `_y_up` 变体 ⇒ **贴图相对几何转 90°**。
- **八面体图里一行会跨过折叠面**，纬度处处不同 ⇒ 纬度必须**逐纹素**算（equirect 一行是一条纬线才可按行平均）。
- `compute_smooth_normals` 在四个角（−Z 极点）给出**零法线**：纹素高度重合、三角形退化成薄片，面法线平均后长度为 0，`try_normalize().unwrap_or(ZERO)` 得到零向量 ⇒ 着色全黑。修法：`grid_normals` 用网格邻接的有限差分算解析法线（`∂r/∂u × ∂r/∂v`，再按半径方向定号），审计变成 `最小点积 0.745` ✓。排查用的硬判据：三角形绕序审计（`203522 个三角形，0 个面法线朝内`）、位移半径审计（`最远顶点 1.0360` = 1 + 位移上限）、关阴影后**图像字节完全没变**、**`unlit` 对照贴图完全正确 ⇒ 锁定法线**。

### §28.2 源码哈希有个洞

算子只哈希**自己那个文件**（`include_str!("fbm.rs")`）⇒ 改**共享代码**（`field.rs` 里的方向约定、`noise.rs`）时，§19.1 那条「源码变了」警告**不会响**；当时手动把 `field.fbm` / `field.ridged` 升到 **v4**。⚠️ **`SOURCE_HASH` 应该把依赖（`field.rs` / `noise.rs`）也哈希进去** —— 仍是待办。

## §31 换到 cubemap（3×2 图集 + gutter）

### §31.1 布局

一张图集，**3 列 × 2 行**，每格 `cell = face + 2×gutter`（face 256、gutter 2 ⇒ cell 260、图集 **780×520**）。面序 `+X, −X, +Y, −Y, +Z, −Z`。`px_protocol::art` 提供 `cube_direction(face, s, t)` / `cube_face_of(方向)` / `cube_atlas_uv(...)`，**双方共用**（协议拥有格式），并有一条测试钉住「方向↔面内坐标」往返与「gutter 落在邻面贴边处」。

### §31.2 gutter 为什么天然无缝

面的参数化 `direction = f(face, s, t)` 允许 **s、t 超出 [0,1]** —— 外推出去的方向**正好就是邻面的方向** ⇒ gutter 里不需要「复制邻面边缘」这一步特殊处理，照同一个映射烘就行。（`cube_face_of` 的测试证实：越过 +X 面 s<0 的半步，落点确实在 +Z 面的贴边处。）

### §31.4 为什么 cube 赢

八面体的折叠会让**折叠线两侧的相邻纹素对应镜像的不相邻方向**，双线性/各向异性/mip 过滤跨过它就混入无关纹素 ⇒ 棱上必然有线，这是布局的固有性质 ✗。cube 的六个面彼此独立、**没有折叠**，跨面边缘两侧的纹素对应几乎相同的方向，过滤几乎正确，加上 gutter 就完全正确 ✓ —— 这正是天空盒用 cubemap 的道理。`Projection::Octahedral` 与它的映射测试都留着，作为这条推理的证据链；星球图现在走 `Cube`。

## §48 相机表进 `.pxart` + protocol 的产物 diff

### §48.1 落在哪

- `px_protocol::art`：`Camera { direction, distance, tag }`（**局部系**方向 + 以半径 1 为单位的距离）；`AssetManifest.cameras` 与 `AssetManifest.fingerprint`（载荷 FNV-1a，都带 `#[serde(default)]` ⇒ 旧产物照样能读）；`AssetChange` / `AssetDiff` / `Diff` + `fn diff(before, after)`；`read_bundle` / `bundle_of` / `cameras_of`。
- `px_ops`：`GraphSpec.cameras`；`px_ops::cameras::review()` = 那 12 个评审视角；`write_artifact` 写相机表 + 指纹；**相机表进缓存键**（`key_with_cameras`）。
- `px_render`：`planet::camera_for(&Camera)`（`SYSTEM_TILT` 只在这一处出现）；`SheetCell`；`--sheet PNG` / `--columns N`；`accept_jobs` 一次请求 → 一个场景 → N 个视口 → **一张对照图**。

### §48.2 为什么这么切（三条判据）

- **相机表放产物里** ⇒ 「这个产物该怎么看」跟着产物走，`tools/probe.ps1` 的 `$views` + `AimLocal` + `$tilt = 0.34` 不再有第二份副本。产物里存的是**局部方向**，所以 PCG 侧不含任何渲染器约定；倾斜只在 `camera_for` 里出现一次。
- **指纹必须进 manifest**：CAS 路径能分辨「新烘的产物」，但**同名覆盖**（脚本里 hardcode 路径、手工拷文件）分辨不了 —— 参数逐项相同、只有值变了的那个情形，只有指纹能说话。`diff` 的判据顺序因此是：**指纹 > 参数 > 载荷头**。
- **相机表必须进缓存键**：它住在产物里 ⇒ 它变了产物内容就变 ⇒ 不进键就会出现「同一个键、不同内容」（§17.1 的老账）。

### §48.3 数字与注意

- `--sheet` 把 12 个角度从 **12 次请求（12 个进程 + 12 次全量重建 + CPU 拼图）**压成 **1 次请求（1 次重建 + 12 个视口 + 1 张图）**。
- 实现是**一张 target + 每相机一个 `Viewport`**；**只有第 0 台清屏**（后面的给 `ClearColorConfig::None`），否则后画的格子会把前面的擦掉。两台 shader 本来就按 `view.viewport` / `view.world_position` 取值 ⇒ **多相机零 shader 改动**。
- ⚠️ `atmosphere.rs::sync_cameras` 与 `AtmosphereParams.camera_x/y/z` 是死代码：它把「最后一个带 `OrbitCamera` 的相机」写进**全局** material uniform，而 shader 早就改读 `view.world_position` 了 —— 阶段 3 换 surface material 时一起删。
- ⚠️ 协议形状变了 ⇒ 快照变 ⇒ `protocol_hash` 变 ⇒ **旧服务的租约会被握手拒掉**（§13 的设计如此，属预期）。
- ⚠️ 快照 JSON 的键是**排序**的（`serde_json` 没开 `preserve_order`）⇒ 手改必错，一律用 `PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol --test snapshot`。
- ⚠️ `review()` 的两极停在 **82°** 而不是 90°（`looking_at` 的 up 与视线共线会退化）；角度语义从「世界 yaw/pitch + 倾斜」变成「局部方向」，所以对照图构图整体转了 19.5°（`SYSTEM_TILT`），和旧的 `target/probe-*.png` **不可逐像素比**。
- ⚠️ **`game` 目前编译不过**：`game/src/project.rs:17` 还在读 `snapshot.executions`，而 `Snapshot` 已把这个读数换成 `intake`（`game/src/lib.rs:37` 的注释）。`cargo test`（默认 members **含 `game`**）因此是红的；绿的是 `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify`。
