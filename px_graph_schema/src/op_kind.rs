//! **产物档**：一个算子吐的是场、网格还是体积。
//!
//! ⚠ 它决定取向两件事：键里要不要掺评审相机、画布算不算它的分辨率。
//!   两者都可以从它推出来 —— 所以它是**唯一**声明，不再有并行的 `WITH_CAMERAS`。

/// 算子的产物档：决定键里要不要掺评审相机（体积不掺）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Field,
    Mesh,
    Volume,
}
