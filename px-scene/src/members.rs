use std::path::Path;

use px_protocol::scene::Member;

pub fn member_of(graph: &str, node: &str) -> Result<Member, String> {
    let key = px_graph::manifest_key_of(graph, node)?;
    Ok(Member::new(graph, node, &key))
}

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

pub fn path_of(member: &Member, root: &Path) -> Result<std::path::PathBuf, String> {
    member.resolve(root)
}
