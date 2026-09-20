//! 体积算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域。
//!
//! ⚠ **一行实现都没有**：算法在 `px_volume_op` 里（dylib，运行时按身份装载）。
//!   这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、少给一个字段，
//!   都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 图参数的形状（`CloudCoarseInput { coverage }`）**是接口的一部分**，所以它住在声明旁边：
//!   它是"这个算子被接的那个 struct"，不是实现细节。

use px_field_schema::field::Field;
use px_graph_schema::{Cooked, px_op};
use px_protocol::art::TextureData;

use crate::VolumeData;
use crate::params;

/// 烘一份体积要吃的东西：**一张覆盖度场**。
#[derive(px_derive::PxInputs)]
pub struct CloudCoarseInput {
    pub coverage: Cooked<Field>,
}

/// 烘一份**可步进的密度**要吃的东西：**一张三维密度场**（`Domain::Volume`）。
///
/// ⚠ 字段名是 `density` 而不是 `field`：这个算子的语义是"把这张密度场搬进体积"
///   （它是采样源，不是被修改的基底）—— 与 `CloudCoarseInput::coverage` 同一个口径。
#[derive(px_derive::PxInputs)]
pub struct DensityInput {
    pub density: Cooked<Field>,
}

/// 算光照要吃的东西：**一份密度体积**（`cloud.density` 的产物）。
#[derive(px_derive::PxInputs)]
pub struct EmissionInput {
    pub volume: Cooked<VolumeData>,
}

/// 沿视线积分要吃的东西：**一份发射体积** + **一张星图**（`CubeMap`）。
///
/// ⚠ 星图是**图输入**（不是参数）：星点要参与积分（被气遮住、被尘埃染红），所以它必须
///   与发射体积在同一趟里被读到。给成参数的话那张图就没法由别的节点造出来。
#[derive(px_derive::PxInputs)]
pub struct SkyInput {
    pub volume: Cooked<VolumeData>,
    pub stars: Cooked<Field>,
}

/// 产物形状：`cook::<CloudCoarse>` 返回的就是它。
///
/// ⚠ 图脚本那一侧读体积的判据仪器都拿这个别名当签名（它只说明"拿到手的是一份体积"）。
pub type VolumeOut = Cooked<VolumeData>;

px_op! {
    /// 烘一份体积（立方球参数空间）。
    CloudCoarse, "cloud.coarse", "px_volume_op", params::Params, CloudCoarseInput, VolumeData
}

px_op! {
    /// **三维场 → 可步进的密度体积**。
    ///
    /// ⚠ 与 [`CloudCoarse`] 是**两个算子**（不是同一份参数的新档）：那一档服务**等值面提取**
    ///   （存 `(场-τ)/L`，形状参数是一整套云的形状），这一档服务**体渲染的沿视线积分**
    ///   （存密度本身 + 壳的内外半径，多一个径向保守化）。两者的"值"含义不同。
    Density, "cloud.density", "px_volume_op", params::density::DensityParams, DensityInput, VolumeData
}

px_op! {
    /// **密度体积 → 逐体素的发射与消光**（体渲染的"材质"）。
    ///
    /// ⚠ 光照（含朝光源的遮挡）在这里按**体素**算完，不留给步进：阴影只与"这一点"有关、
    ///   与看它的视线无关 ⇒ 算一遍是 `体素数 × 阴影步数`，塞进步进就是
    ///   `射线数 × 步数 × 阴影步数`。差三个数量级，而那正是"烘图时步进"能成立的全部理由。
    Emission, "cloud.emission", "px_volume_op", params::emission::EmissionParams, EmissionInput, VolumeData
}

px_op! {
    /// **沿视线积分**：发射体积 + 星图 → **一张天空立方贴图**（HDR）。
    ///
    /// ⚠ 三条通道在**这一档内部**各积一遍（参数里没有 `channel`）：逐通道消光意味着三张
    ///   本来就不同，一次交出一张贴图才是这个算子该有的形状
    ///   —— 分成三个节点会让"一张天空"变成三个必须自己对齐的产物。
    ///
    /// ⚠ 它**只收发射体积**（不是密度场）：搬密度、算光照是 `cloud.density` /
    ///   `cloud.emission` 那两档的事，而它们与"壳多细、半径多大"有关、与"积多细"无关
    ///   ⇒ 那些参数不该出现在这一档（出现就是多一处必须与上游一致的抄写）。
    ///
    /// ⚠ 输出是**贴图**：`TextureData` 的载荷类型与它的 `Build` 都住在 `px_protocol::art` /
    ///   `px_protocol::payload`（与 `VolumeData` / `MeshData` 同一个口径）⇒ 天空是一条
    ///   **正常的图产物**，场景文档按 `"图名::节点名"` 引用它当 `environment.skybox`，
    ///   渲染器一个字节不用改。
    SkyNebula, "sky.nebula", "px_volume_op", params::sky::SkyParams, SkyInput, TextureData
}
