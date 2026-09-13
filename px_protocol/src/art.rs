use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::wire::{Blob, BlobHeader, WireError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Field2D,
    OctahedralField,
    CubeField,
    CubeMap,
    Mesh,
    Instances,
}

fn sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

pub fn octahedral_direction(u: f32, v: f32) -> [f32; 3] {
    let x = u.clamp(0.0, 1.0) * 2.0 - 1.0;
    let y = v.clamp(0.0, 1.0) * 2.0 - 1.0;
    let mut direction = [x, y, 1.0 - x.abs() - y.abs()];
    if direction[2] < 0.0 {
        let (old_x, old_y) = (direction[0], direction[1]);
        direction[0] = (1.0 - old_y.abs()) * sign(old_x);
        direction[1] = (1.0 - old_x.abs()) * sign(old_y);
    }
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn octahedral_direction_y_up(u: f32, v: f32) -> [f32; 3] {
    let direction = octahedral_direction(u, v);
    [direction[0], direction[2], -direction[1]]
}

pub fn octahedral_uv(direction: [f32; 3]) -> [f32; 2] {
    let length = direction[0].abs() + direction[1].abs() + direction[2].abs();
    if length <= f32::EPSILON {
        return [0.5, 0.5];
    }
    let mut x = direction[0] / length;
    let mut y = direction[1] / length;
    if direction[2] < 0.0 {
        let old_x = x;
        x = (1.0 - y.abs()) * sign(old_x);
        y = (1.0 - old_x.abs()) * sign(y);
    }
    [x * 0.5 + 0.5, y * 0.5 + 0.5]
}

pub fn octahedral_uv_y_up(direction: [f32; 3]) -> [f32; 2] {
    octahedral_uv([direction[0], -direction[2], direction[1]])
}

pub const MESH_ATTRIBUTES: [&str; 4] = ["positions", "normals", "uvs", "indices"];
pub const MESH_POSITION: usize = 0;
pub const MESH_NORMAL: usize = 1;
pub const MESH_UV: usize = 2;
pub const MESH_INDEX: usize = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshData {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn vertices(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn blobs(&self) -> Vec<Blob> {
        vec![
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.positions),
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.normals),
            Blob::from_f32(vec![self.vertices() as u32, 2], &self.uvs),
            Blob::from_u32(vec![self.indices.len() as u32], &self.indices),
        ]
    }

    pub fn from_blobs(blobs: &[&Blob]) -> Result<Self, WireError> {
        if blobs.len() < MESH_ATTRIBUTES.len() {
            return Err(WireError::TruncatedFrame);
        }
        let mesh = Self {
            positions: blobs[MESH_POSITION].f32s()?,
            normals: blobs[MESH_NORMAL].f32s()?,
            uvs: blobs[MESH_UV].f32s()?,
            indices: blobs[MESH_INDEX].u32s()?,
        };
        let vertices = mesh.vertices();
        if mesh.normals.len() != vertices * 3 || mesh.uvs.len() != vertices * 2 {
            return Err(WireError::TruncatedFrame);
        }
        Ok(mesh)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub id: String,
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<BlobHeader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ArtBundle {
    pub assets: Vec<AssetManifest>,
}

pub const CUBE_FACES: u32 = 6;
pub const CUBE_COLUMNS: u32 = 3;
pub const CUBE_GUTTER: u32 = 2;

pub fn cube_cell_size(width: u32) -> u32 {
    width / CUBE_COLUMNS
}

pub fn cube_face_size(width: u32) -> u32 {
    cube_cell_size(width).saturating_sub(CUBE_GUTTER * 2)
}

pub fn cube_direction(face: u32, s: f32, t: f32) -> [f32; 3] {
    let a = s * 2.0 - 1.0;
    let b = t * 2.0 - 1.0;
    let direction = match face % CUBE_FACES {
        0 => [1.0, -b, -a],
        1 => [-1.0, -b, a],
        2 => [a, 1.0, b],
        3 => [a, -1.0, -b],
        4 => [a, -b, 1.0],
        _ => [-a, -b, -1.0],
    };
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn cube_face_of(direction: [f32; 3]) -> (u32, f32, f32) {
    let (x, y, z) = (direction[0], direction[1], direction[2]);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let face = if ax >= ay && ax >= az {
        if x > 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if y > 0.0 { 2 } else { 3 }
    } else if z > 0.0 {
        4
    } else {
        5
    };
    let major = match face {
        0 | 1 => ax,
        2 | 3 => ay,
        _ => az,
    }
    .max(f32::EPSILON);
    let (a, b) = match face {
        0 => (-z, -y),
        1 => (z, -y),
        2 => (x, z),
        3 => (x, -z),
        4 => (x, -y),
        _ => (-x, -y),
    };
    let s = (a / major) * 0.5 + 0.5;
    let t = (b / major) * 0.5 + 0.5;
    (face, s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))
}

pub fn cube_atlas_uv(face: u32, s: f32, t: f32, face_size: u32, gutter: u32) -> [f32; 2] {
    let cell = face_size + gutter * 2;
    let column = face % CUBE_COLUMNS;
    let row = face / CUBE_COLUMNS;
    let x = (column * cell) as f32 + gutter as f32 + s * face_size as f32;
    let y = (row * cell) as f32 + gutter as f32 + t * face_size as f32;
    let width = (cell * CUBE_COLUMNS) as f32;
    let height = (cell * 2) as f32;
    [x / width, y / height]
}


pub fn cube_map_extent(face_size: u32) -> (u32, u32) {
    let face = face_size.max(1);
    (face, face * CUBE_FACES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Domain {
    Equirect,
    Octahedral,
    Cube,
    CubeMap,
}

impl Domain {
    pub fn name(self) -> &'static str {
        match self {
            Self::Equirect => "equirect",
            Self::Octahedral => "octahedral",
            Self::Cube => "cube",
            Self::CubeMap => "cubemap",
        }
    }
}

pub fn direction_at(
    domain: Domain,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> [f32; 3] {
    let u = (x as f32 + 0.5) / width.max(1) as f32;
    let v = (y as f32 + 0.5) / height.max(1) as f32;
    match domain {
        Domain::Equirect => {
            let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
            let phi = u * std::f32::consts::TAU;
            let ring = theta.sin();
            [ring * phi.cos(), theta.cos(), ring * phi.sin()]
        }
        Domain::Octahedral => octahedral_direction_y_up(u, v),
        Domain::Cube => {
            let cell = cube_cell_size(width).max(1);
            let face_size = cube_face_size(width).max(1);
            let gutter = CUBE_GUTTER;
            let face = (y / cell) * CUBE_COLUMNS + (x / cell);
            let s = (x % cell) as f32 + 0.5 - gutter as f32;
            let t = (y % cell) as f32 + 0.5 - gutter as f32;
            cube_direction(face, s / face_size as f32, t / face_size as f32)
        }
        Domain::CubeMap => {
            let face_size = width.max(1);
            let face = (y / face_size).min(CUBE_FACES - 1);
            let s = (x as f32 + 0.5) / face_size as f32;
            let t = ((y % face_size) as f32 + 0.5) / face_size as f32;
            cube_direction(face, s, t)
        }
    }
}

pub fn uv_of(domain: Domain, direction: [f32; 3], width: u32, _height: u32) -> [f32; 2] {
    match domain {
        Domain::Equirect => {
            let v = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            let u = (direction[2].atan2(direction[0]) / std::f32::consts::TAU).rem_euclid(1.0);
            [u, v]
        }
        Domain::Octahedral => octahedral_uv_y_up(direction),
        Domain::Cube => {
            let (face, s, t) = cube_face_of(direction);
            cube_atlas_uv(
                face,
                s,
                t,
                cube_face_size(width),
                CUBE_GUTTER,
            )
        }
        Domain::CubeMap => {
            let (face, s, t) = cube_face_of(direction);
            [s, (face as f32 + t) / CUBE_FACES as f32]
        }
    }
}
