// 网格顶点阶段（帧图的几何 pass 用它，§128）。
//
// ⚠ 它与内容材质的片元阶段之间的接口是 **`VertexOutput`**，字段/origin/次序必须逐字对上
// （`px_render_wgpu/src/material.rs` 的 `VERTEX_PROBE` 是同一份契约的另一处转写）：
//
//     struct VertexOutput {
//         @builtin(position) position: vec4<f32>,
//         @location(0) world_position: vec4<f32>,
//         @location(1) world_normal: vec3<f32>,
//         @location(2) uv: vec2<f32>,
//     };
//
// ⚠ 第一版这里**只写了 `@builtin(position)`**（§128 的初稿），而 `surface.wgsl` /
// `atmosphere.wgsl` 的片元读 `in.uv` / `in.world_normal` / `in.world_position` /
// `in.position` —— wgpu 会当场拒建管线（"Location[0] … is not provided by the
// previous stage outputs"）。**少写三个输出不是"先简陋一点"，是这条 pass 根本建不起来。**
//
// 为什么顶点阶段由宿主给、而不是执行器自带：内容 shader 是**纯片元**的
// （pxart 那几份没有 `@vertex`），顶点变换是宿主与 Bevy **逐位对齐**的那一段（§110）。
//
// ⚠ 矩阵的**位模式**由宿主负责（`px_render_wgpu/src/mat4.rs` 那几份移植件）：
// 这里只做乘法，不做任何化简（`view_proj * (world * p)` 的括号也要留住 ——
// 换成 `(view_proj * world) * p` 就是另一个数）。

// ============================================================================
// 组 1：**两类参数**（§142 的用户裁决：per pass 的是 material 参数的 super class）
//
// 这里原来是一个 `MeshStage`（三块矩阵绑成一份 176 B，每个 (物体, 视图) 各一份）。
// 现在按**参数是谁在配**拆成两格：
//
//   binding 0  `PassView`     —— **super**：这一条 pass 的 `view_proj`，**一条 pass 一份**。
//                                由文档的 `passes[]` 说了算（`cube_face{light,face,layer}`
//                                → 那一面；没有 cube_face 的就是相机），宿主把它解析成
//                                一个 64 B 的 uniform。
//   binding 1  `MeshInstance` —— **instance**：**长度等于物体数的一份 buffer of struct**，
//                                靠 `@builtin(instance_index)` 选这一笔画的是哪一格；
//                                下标 = 物体在 `objects[]` 里的次序（宿主与执行器都不认识
//                                "物体"，它们只是把同一个表的两半对上了）。
//
// ⚠ **为什么能拆**：`world_from_local` 与法线矩阵**与视图无关**（它们是物体的），
//    只有 `view_proj` 是每视图的。拆开之后那块数据从
//    「(物体 × 视图) × 176 B」变成「物体 × 112 B + 视图 × 64 B」——
//    而这不是省字节：是**把"谁在索引它"这件事说清楚了**（super 由 pass 直接给，
//    instance 由下标选）。
//
// ⚠⚠ 实例化**改变绘制调用本身**：它与 §109.1 记的 multiview 同属
//    「**行为差异，不是等价实现**」⇒ 必须**证明**逐位等价，不许假设它等价。
//    今天每一笔给的是一格宽的区间 `k..k+1`，于是 `instance_index` 恒为 `k`，
//    每一个算式与拆之前逐位相同 —— 判据就是那六档整份 PNG 的哈希。
//
// ⚠ 读法照旧逐字保留：`view_proj * (world * p)`，括号一步都不许省。
// ============================================================================

struct PassView {
    /// 这一条 pass 的 `clip_from_world`（相机那一份，或者 cube 某一面的那一份）。
    view_proj: mat4x4<f32>,
};

struct MeshInstance {
    world_from_local: mat4x4<f32>,
    /// 法线矩阵 = `Affine3A::inverse().matrix3.transpose()`（Bevy 的
    /// `local_from_world_transpose`，见下面 `vertex` 里那段）。
    normal: mat3x3<f32>,
};

@group(1) @binding(0) var<uniform> pass_view: PassView;
// ⚠ `storage`（不是 `uniform`）：oracle 那边这份每实例数据就是
// `var<storage> mesh: array<Mesh>`（`bevy_pbr-0.19.1/src/render/mesh_bindings.wgsl:9`）。
// 两边都是"按 `instance_index` 选一格"，取哪一档地址空间**不改任何一条算术**，
// 但跟着 oracle 走免得下一次对账时先怀疑这里。
@group(1) @binding(1) var<storage, read> mesh: array<MeshInstance>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // ⚠ 那个"索引"就在这里：宿主与执行器都不解释它，是**顶点阶段**拿它去选格。
    //    （所以"per-object = 按下标选一格"这件事只在 shader 里成立，不在执行器的词汇里。）
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    // ⚠ 三处输出与 oracle 的对应关系（§110）：世界位置是 `world_from_local × p`；
    //    世界法线走**法线矩阵**并且**逐顶点归一化**（见下）；`uv` 原样透传。
    let world = mesh[instance_index].world_from_local * vec4<f32>(position, 1.0);
    var out: VertexOutput;
    out.position = pass_view.view_proj * world;
    out.world_position = world;
    // ⚠⚠ 法线这一格曾经写成 `(world_from_local * vec4(normal, 0.0)).xyz`，理由是
    //    "这几档的缩放是均匀的 1.0 ⇒ 3×3 就是正确的法线矩阵"。那句**数学上对、
    //    逐位上错**：Bevy 那条路（`bevy_pbr-0.19.1/src/render/mesh.wgsl:64-69` →
    //    `mesh_functions.wgsl:68-84`）用的是 `local_from_world_transpose`，而它是
    //    `Affine3A::from(world_from_local).inverse().matrix3.transpose()`
    //    （`bevy_math-0.19.1/src/affine3.rs:37-43`）—— 一个**专用例程**，与"取 3×3"
    //    不是同一串算术。另外它**在顶点阶段就归一化**（`mesh_functions.wgsl:77`），
    //    而我们原来是插值之后才在片元里归一化 —— `normalize(lerp(a,b))` 与
    //    `lerp(normalize(a), normalize(b))` 也是两个数。
    //    两处都属于"数学等价、浮点不等价"那一族，症状一模一样：**只进着色、不进位置
    //    /深度** ⇒ 盘内散落的 ±1，而轮廓、深度、背景全都好。
    //    归一化那一步照抄 Bevy 的守卫：法线全零就**原样给出去**（不让它变成 NaN）。
    out.world_normal = select(
        normal,
        normalize(mesh[instance_index].normal * normal),
        any(normal != vec3<f32>(0.0)),
    );
    out.uv = uv;
    return out;
}
