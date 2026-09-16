//! **oracle 导出**：把 Bevy 现场生成的图元网格原样落盘，给 `px_render_wgpu` 的移植当判据。
//!
//! 为什么要有这个文件：`px_render_wgpu` 不许依赖 bevy（那正是剥离的目的），
//! 于是它必须**自己**把 `Sphere::new(r).mesh().ico(n)` 生成的那份网格造出来。
//! "自己造一份长得差不多的"在逐字节判据上不算数 —— 所以先把 Bevy 那份**存下来当锚**，
//! 再拿新宿主那份与它比（`target/oracle/check-primitive.ps1`）。
//!
//! ⚠ 默认 `#[ignore]`：它不是门（门是 `cargo test`），而是一次性的导出。
//! 用法：`cargo test -p px_render --test primitive_oracle -- --ignored --nocapture`
//!
//! 落盘格式（小端，按这个顺序拼，不含任何头部）：
//!   positions: n × 3 × f32 ｜ normals: n × 3 × f32 ｜ uvs: n × 2 × f32 ｜ indices: m × u32
//! 文件哈希就是判据 —— 比逐行文本稳，也比"看数量对不对"强。

use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::prelude::*;

fn write_mesh(tag: &str, mesh: &Mesh) {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        other => panic!("{tag} 的 POSITION 不是 Float32x3：{other:?}"),
    };
    let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        other => panic!("{tag} 的 NORMAL 不是 Float32x3：{other:?}"),
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(values)) => values.clone(),
        other => panic!("{tag} 的 UV_0 不是 Float32x2：{other:?}"),
    };
    let indices = match mesh.indices() {
        Some(Indices::U32(values)) => values.clone(),
        other => panic!("{tag} 的索引不是 U32：{other:?}"),
    };

    let mut bytes: Vec<u8> = Vec::new();
    for position in &positions {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for normal in &normals {
        for value in normal {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for uv in &uvs {
        for value in uv {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in &indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }

    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render 必须住在 workspace 下");
    let directory = workspace.join("target").join("oracle");
    std::fs::create_dir_all(&directory).expect("建不了 target/oracle");
    let path = directory.join(format!("bevy-{tag}.bin"));
    std::fs::write(&path, &bytes).expect("写不了 oracle");

    println!(
        "{tag}：顶点 {}｜三角形 {}｜字节 {}｜{}",
        positions.len(),
        indices.len() / 3,
        bytes.len(),
        path.display()
    );
}

/// 细分球：`orbit-bare` 的大气与 `orbit-soft` 的云壳都走这一条（`mesh.rs::primitive`）。
#[test]
#[ignore = "oracle 导出：跑一次就够，不进日常门"]
fn dump_the_icospheres_the_scenes_use() {
    for subdivisions in [1_u32, 5, 64] {
        let mesh = Sphere::new(1.0)
            .mesh()
            .ico(subdivisions)
            .unwrap_or_else(|err| panic!("ico({subdivisions}) 造不出来：{err}"));
        write_mesh(&format!("icosphere-r1.0-s{subdivisions}"), &mesh);
    }
    // 半径也进文件名：产物给的是 (radius, subdivisions) 两个数，两个都要钉住。
    for (radius, subdivisions) in [(1.06_f32, 64_u32), (1.02, 64)] {
        let mesh = Sphere::new(radius)
            .mesh()
            .ico(subdivisions)
            .unwrap_or_else(|err| panic!("ico 造不出来：{err}"));
        write_mesh(&format!("icosphere-r{radius}-s{subdivisions}"), &mesh);
    }
}

/// UV 球：`uv_sphere` 那一条（四档场景今天没用到，留着防以后接上时无从对照）。
#[test]
#[ignore = "oracle 导出：跑一次就够，不进日常门"]
fn dump_the_uv_spheres() {
    for (sectors, stacks) in [(32_u32, 18_u32), (64, 32)] {
        let mesh = Sphere::new(1.0).mesh().uv(sectors, stacks);
        write_mesh(&format!("uvsphere-r1.0-{sectors}x{stacks}"), &mesh);
    }
}
