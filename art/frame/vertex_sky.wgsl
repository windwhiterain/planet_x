// 天空盒顶点阶段（帧图的 `sky` 那条 pass 用它，§128）。
//
// 这一笔是**程序化**的：没有顶点缓冲（`ResolvedGeometry.vertices = None`），
// 三个顶点由 `vertex_index` 现算 —— 这就是判据里那条"没有顶点缓冲也画得出来"的路。
//
// 与 oracle 的对应：Bevy 的天空盒也是一个盖住屏幕的三角，片元里用
// `view_from_clip` / `world_from_view` / `viewport` 反算出射线方向（§109）。
// 这里把**同一个**裁剪坐标交给片元（`@location(0)`），由天空盒材质的片元阶段
// 去反算方向 —— 顶点阶段只负责把三个顶点摆到屏幕外沿。

struct Out {
    @builtin(position) position: vec4<f32>,
    // 裁剪空间坐标，原样交给片元（片元里再用逆矩阵反算世界系射线方向）。
    @location(0) clip: vec4<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Out {
    // 一个盖满屏幕的大三角：(-1,-1) / (3,-1) / (-1,3)。
    // ⚠ 顶点顺序与下标表一一对应，换顺序就是换绕向。
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: Out;
    // 天空盒在**远平面**上：reverse-Z 的远端是 0.0，正好让深度测试放它过去，
    // 而又被任何写过的深度挡住（`greater_equal`）。
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.clip = out.position;
    return out;
}
