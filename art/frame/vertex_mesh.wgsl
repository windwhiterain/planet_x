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
// ⚠ 与宿主之间的约定是 **group 1 binding 0**：每个物体一份 `MeshStage`
// （世界系 ↔ 局部系 + 相机矩阵）。宿主按 `Draw.geometry` 那个名字（物体 id）查出它的
// 网格与变换，再把这两块矩阵填进来。
//
// 为什么顶点阶段由宿主给、而不是执行器自带：内容 shader 是**纯片元**的
// （pxart 那几份没有 `@vertex`），顶点变换是宿主与 Bevy **逐位对齐**的那一段（§110）。
//
// ⚠ 矩阵的**位模式**由宿主负责（`px_render_wgpu/src/mat4.rs` 那几份移植件）：
// 这里只做乘法，不做任何化简（`view_proj * (world * p)` 的括号也要留住 ——
// 换成 `(view_proj * world) * p` 就是另一个数）。

struct MeshStage {
    world_from_local: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    /// 法线矩阵 = `Affine3A::inverse().matrix3.transpose()`（Bevy 的
    /// `local_from_world_transpose`，见下面 `vertex` 里那段）。
    normal: mat3x3<f32>,
};

@group(1) @binding(0) var<uniform> stage: MeshStage;

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
) -> VertexOutput {
    // ⚠ 三处输出与 oracle 的对应关系（§110）：世界位置是 `world_from_local × p`；
    //    世界法线走**法线矩阵**并且**逐顶点归一化**（见下）；`uv` 原样透传。
    let world = stage.world_from_local * vec4<f32>(position, 1.0);
    var out: VertexOutput;
    out.position = stage.view_proj * world;
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
        normalize(stage.normal * normal),
        any(normal != vec3<f32>(0.0)),
    );
    out.uv = uv;
    return out;
}
