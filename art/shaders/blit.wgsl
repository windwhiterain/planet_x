//! 帧图的最后一步：把中间目标**搬到**最终目标。
//!
//! oracle 的最后一步也是 blit（主纹理 → 输出），不是调色 —— 这条 pass 的职责
//! 就是**搬**。所以它有自己这支 shader，而不是借 `px_grade` 加一个 `strength = 0`：
//! 借来的话，配方描述的就是"一道调色"，而谁在查一个错的像素时，应该读六行就能回答
//! "是不是 blit 的问题"，不必先去想"grade 取 0 的时候到底等不等于恒等"。
//!
//! 绑定契约与材质同一条：第 0 格是自己声明的参数块，贴图从第 1 格起
//! （采样器在 `+1`）。`#{MATERIAL_BIND_GROUP}` 由组装器替成运行期那个数。

struct PxBlitParams {
    /// 1.0 = 逐位搬运（`x * 1.0 == x`，有限值、±0、±inf 都成立）。
    ///
    /// ⚠ 留这一格不是装饰：`px_pass` 要求全屏 pass 的参数块**非空**（执行器总是绑着它，
    /// 0 字节当场拒）。既然必须有一个数，就给一个**真的用得上**的：乘法恒等，
    /// 而不是一格没人读的占位（"说了没做"那一类）。
    gain: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PxBlitParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var px_source: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var px_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(px_source, px_sampler, uv) * params.gain;
}
