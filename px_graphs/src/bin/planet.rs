//! 星球：**普通 Rust** —— 一张场接一张场地算下去，每一步走 `px_cook` 那个缓存辅助函数。
//!
//! ⚠ 这里原来是老写法（`node(params::FBM, "continents", &[])`：字符串 id + 一个
//! `&[&Artifact]` 字节边界，类型全擦除）。现在接错一个输入、少给一个上游都是**编译错**。

use px_cook::cook;
use px_field_op::typed as field;
use px_graph::{GraphSpec, begin, finish};
use px_mesh_op::typed as mesh;
use px_protocol::art::Domain;

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    begin(GraphSpec {
        name: "planet".to_string(),
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: px_graph::cameras::review(),
    });
    let cache = px_graph::driver();

    let continents = cook::<field::Fbm>(&cache, "continents", ())?;
    let mountains = cook::<field::Ridged>(&cache, "mountains", ())?;
    let weight = cook::<field::Constant>(&cache, "weight", ())?;
    let terrain = cook::<field::Mix>(
        &cache,
        "terrain",
        field::MixInput {
            a: continents,
            b: mountains,
            mask: weight,
        },
    )?;
    let height = cook::<field::Remap>(
        &cache,
        "height",
        field::FieldInput {
            field: terrain.clone(),
        },
    )?;

    let surface = cook::<mesh::CubeSphere>(
        &cache,
        "surface",
        mesh::CubeSphereInput {
            height: height.clone(),
        },
    )?;

    let stats = height.value().stats();
    println!(
        "输出 height：{}×{}，值域 {:.4}..{:.4}，均值 {:.4}",
        height.value().width,
        height.value().height,
        stats.min,
        stats.max,
        stats.mean,
    );
    println!(
        "输出 surface：{} 顶点 / {} 三角形",
        surface.mesh().vertices(),
        surface.mesh().triangles()
    );

    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员（要在 art/scene/ 里自己接上）：height {}｜surface {}",
        px_graph::artifact_path_of(&height.key).display(),
        px_graph::artifact_path_of(&surface.key).display(),
    );

    finish();
    Ok(())
}
