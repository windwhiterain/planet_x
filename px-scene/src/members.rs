//! **成员解析**：配方里写的是「图名 :: 节点名」这种给人看的引用，产物里要的是**键**。
//!
//! 这一层是那份"名字 → 键"的唯一落点（`px_ops::manifest_key_of`）：场景编译器自己不去
//! 拼路径、不去读索引 —— 走另一条路就是第二个会漂开的真相（§17.3 的清单是唯一可读出口）。

use std::path::Path;

use px_protocol::scene::Member;

/// 按图名 + 节点名取成员（键从那份图的清单里来）。
pub fn member_of(graph: &str, node: &str) -> Result<Member, String> {
    let key = px_ops::manifest_key_of(graph, node)?;
    Ok(Member::new(graph, node, &key))
}

/// 解析一条**配方里的引用**：`"图名::节点名"`，或者裸名字（那时用 `graph` 给的默认图）。
///
/// `what` 只进报错（`part 'planet' 的成员 'height'`）。
pub fn reference(
    what: &str,
    reference: &str,
    default_graph: Option<&str>,
) -> Result<Member, String> {
    match reference.split_once("::") {
        Some((graph, node)) => member_of(graph, node).map_err(|err| format!("{what}：{err}")),
        None => {
            let graph = default_graph.ok_or_else(|| {
                format!(
                    "{what} 没说属于哪张图：要么给 part.graph，要么把引用写成 图名::节点名\
                     （今天是 '{reference}'）"
                )
            })?;
            member_of(graph, reference).map_err(|err| format!("{what}：{err}"))
        }
    }
}

/// 一份**图产品**（场 / 网格 / 贴图）的路径：由成员键解出来。
pub fn path_of(member: &Member, root: &Path) -> Result<std::path::PathBuf, String> {
    member.resolve(root)
}
