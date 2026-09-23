//! 驱动与 `cached` 之间**唯一的接缝**。
//!
//! `cached`（`px_cook`）只认识这几个方法；驱动（`px_graph`）实现它。
//! 两边都不需要知道对方的算子长什么样 —— 所以这一层必须比驱动低。

use px_protocol::art::Camera;

use crate::payload::PayloadBundle;
use crate::{Grid, Key};

/// **缓存机制**：图脚本那边的类型化门面（`px_cook`）只认这一个接口。
///
/// ⚠ 它**不认识任何算子** —— 报上下文、读参数原文、查/写 CAS、记读数，就这四件事。
pub trait Cache {
    /// **画布 = 算子拿到的那个 `Grid`**（尺寸 + 投影）。
    ///
    /// ⚠ 只有一个出口：键里那一份与算子 `render` 手里那一份必须是同一个值。
    ///   从前它们是两处（`canvas()` + `projection()` 给键、`cached` 的参数给算子）——
    ///   那种"双份真相"是错的。
    fn grid(&self) -> Grid;
    fn cameras(&self) -> &[Camera];
    /// `art/<图>/<name>.toml` 的原文；`None` = 文件不存在 ⇒ 用算子默认值。
    fn params_text(&self, name: &str) -> Option<String>;
    /// 记一条"这个节点读了哪些参数"（诊断用：写进 `<图>/params.json`）。
    ///
    /// ⚠ 缺文件是**静默用默认值**的（老口径，不动）—— 于是设计师看不到自己少写了什么。
    /// 这一条记录就是那个缺口：跑完能拿到"每个节点实际生效的参数值 + 它的字段名"。
    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool);
    /// CAS 里那份**载荷**。`PX_PCG_FRESH=1` 时一律 `None`（本次全部重算）。
    ///
    /// ⚠ 回的是解开的 `PayloadBundle`（不是裸字节）：命中路径本来就要解一次，
    ///   从前驱动与 `decode` 各解一次 —— 同一份 26 MB 的体积被解析了两遍。
    fn fetch(&self, key: Key) -> Option<PayloadBundle>;
    /// 键不在盘上：把产物写进 CAS、记清单，并把读数打出来。
    ///
    /// ⚠ `id`（节点名）与相机表**只在这里**补上 —— 算子交出来的载荷是无名、无相机的。
    fn store(&self, report: Report<'_>, payload: &PayloadBundle) -> Result<(), String>;
}

/// 一次 cached 的读数 —— 与 `cached` 打出来的那几行同一档。
pub struct Report<'a> {
    pub node: &'a str,
    pub op: &'static str,
    /// **接口形状哈希**（完整 64 位）—— 它取代了手写的 `version`。
    pub interface: u64,
    pub key: Key,
    pub hit: bool,
    pub millis: u64,
    /// 体积那一档不掺评审相机。
    pub with_cameras: bool,
    /// 这份产物"长什么样"，**由域自己说**（类型化的值在它手里，不必让驱动解字节去猜）。
    pub detail: String,
}
