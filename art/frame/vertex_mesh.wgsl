// 网格顶点阶段（帧图的几何 pass 用它，§128）。
//
// ⚠ 这一段与宿主之间的约定是 **group 1 binding 0**：每个物体一份
// `MeshStage`（世界系 ↔ 局部系 + 相机矩阵）。宿主按 `Draw.geometry` 那个名字
// （物体 id）查出它的网格与变换，再把这两块矩阵填进来。
//
// 为什么顶点阶段由宿主给、而不是执行器自带：内容 shader 是**纯片元**的
// （pxart 那几份没有 `@vertex`），顶点变换是宿主与 Bevy **逐位对齐**的那一段（§110）。
// 执行器要是自己写一份"差不多"的，两条宿主就会在两套矩阵算法上分岔 ——
// 而逐字节判据最怕的正是那种漂移。
//
// ⚠ 矩阵的**位模式**由宿主负责（`px_render_wgpu/src/mat4.rs` 那几份移植件）：
// 这里只做两次乘法，不做任何化简（`view_proj * (world * p)` 的括号也要留住 ——
// 换成 `(view_proj * world) * p` 就是另一个数）。

struct MeshStage {
    world_from_local: mat4x4<f32>,
    view_proj: mat4x4<f32>,
};

@group(1) @binding(0) var<uniform> stage: MeshStage;

struct Out {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vertex(@location(0) position: vec3<f32>) -> Out {
    var out: Out;
    out.position = stage.view_proj * (stage.world_from_local * vec4<f32>(position, 1.0));
    return out;
}
