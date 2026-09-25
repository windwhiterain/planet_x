use crate::assemble::{HOST_VIEW_STUB, bevy_stub};

pub const POINT_SHADOW_STUB: &str = "\
@group(0) @binding(2) var point_shadow_textures: texture_depth_2d_array;\n\
@group(0) @binding(22) var point_shadow_textures_l1: texture_depth_2d_array;\n\
@group(0) @binding(23) var point_shadow_textures_l2: texture_depth_2d_array;\n\
@group(0) @binding(24) var point_shadow_textures_l3: texture_depth_2d_array;\n\
@group(0) @binding(3) var point_shadow_textures_comparison_sampler: sampler;\n\
@group(0) @binding(4) var<storage, read> px_shadow_pages: array<u32>;\n\
@group(0) @binding(5) var<storage, read> px_shadow_light_offsets: array<u32>;\n\
\n\
fn copysign(a: f32, b: f32) -> f32 {\n\
\x20   return bitcast<f32>((bitcast<u32>(a) & 0x7FFFFFFF) | (bitcast<u32>(b) & 0x80000000));\n\
}\n\
\n\
fn orthonormalize(z_basis: vec3<f32>) -> mat3x3<f32> {\n\
\x20   let sign = copysign(1.0, z_basis.z);\n\
\x20   let a = -1.0 / (sign + z_basis.z);\n\
\x20   let b = z_basis.x * z_basis.y * a;\n\
\x20   let x_basis = vec3<f32>(1.0 + sign * z_basis.x * z_basis.x * a, sign * b, -sign * z_basis.x);\n\
\x20   let y_basis = vec3<f32>(b, sign + z_basis.y * z_basis.y * a, -z_basis.y);\n\
\x20   return mat3x3<f32>(x_basis, y_basis, z_basis);\n\
}\n\
\n\
const PX_D3D_SAMPLE_POINT_POSITIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(\n\
\x20   vec2<f32>( 0.125, -0.375),\n\
\x20   vec2<f32>(-0.125,  0.375),\n\
\x20   vec2<f32>( 0.625,  0.125),\n\
\x20   vec2<f32>(-0.375, -0.625),\n\
\x20   vec2<f32>(-0.625,  0.625),\n\
\x20   vec2<f32>(-0.875, -0.125),\n\
\x20   vec2<f32>( 0.375,  0.875),\n\
\x20   vec2<f32>( 0.875, -0.875),\n\
);\n\
const PX_D3D_SAMPLE_POINT_COEFFS: array<f32, 8> = array<f32, 8>(\n\
\x20   0.157112, 0.157112, 0.138651, 0.130251, 0.114946, 0.114946, 0.107982, 0.079001,\n\
);\n\
\n\
const PX_PAGE_SIZE: u32 = 128u;\n\
const PX_PAGE_BITS: u32 = 7u;
const PX_CUBE_FACES: u32 = 6u;\n\

\n\
@group(0) @binding(6) var<storage, read> px_shadow_faces: array<vec4<f32>>;\n\
\n\
fn px_shadow_face_basis(face: u32) -> mat3x3<f32> {\n\
\x20   let at = face * 3u;\n\
\x20   return mat3x3<f32>(\n\
\x20       px_shadow_faces[at].xyz,\n\
\x20       px_shadow_faces[at + 1u].xyz,\n\
\x20       px_shadow_faces[at + 2u].xyz,\n\
\x20   );\n\
}\n\
\n\
fn px_shadow_face_of(d: vec3<f32>) -> u32 {\n\
\x20   let a = abs(d);\n\
\x20   if (a.x >= a.y && a.x >= a.z) { return select(1u, 0u, d.x > 0.0); }\n\
\x20   if (a.y >= a.z) { return select(3u, 2u, d.y > 0.0); }\n\
\x20   return select(5u, 4u, d.z > 0.0);\n\
}\n\
\n\
fn px_shadow_face_texel(d: vec3<f32>, face_side: f32) -> vec3<f32> {\n\
\x20   let face = px_shadow_face_of(d);\n\
\x20   let basis = px_shadow_face_basis(face);\n\
\x20   let denom = dot(basis[2], d);\n\
\x20   if (denom == 0.0) { return vec3<f32>(-1.0, -1.0, f32(face)); }\n\
\x20   let u = dot(basis[0], d) / denom;\n\
\x20   let v = dot(basis[1], d) / denom;\n\
\x20   return vec3<f32>(\n\
\x20       (u * 0.5 + 0.5) * face_side,\n\
\x20       (0.5 - v * 0.5) * face_side,\n\
\x20       f32(face),\n\
\x20   );\n\
}\n\
fn px_shadow_page_slot(light_id: u32, level: u32, face: u32, page_x: u32, page_y: u32) -> i32 {\n\
\x20   let off = px_shadow_light_offsets[light_id];\n\
\x20
\x20
\x20
\x20
\x20   let head = px_shadow_pages[off];\n\
\x20   let pps0 = max(head & 0xFFFFu, 1u);\n\
\x20   let levels = max(px_shadow_pages[off + 1u] & 0xFFFFu, 1u);\n\
\x20   if (level >= levels) { return -1; }\n\
\x20   let pps = max(pps0 >> level, 1u);\n\
\x20   if (page_x >= pps || page_y >= pps) { return -1; }\n\
\x20   let row_words = 1u + ((pps + 31u) >> 5u);\n\
\x20   let base = off + px_shadow_pages[off + 2u + level]\n\
\x20       + face * pps * row_words + page_y * row_words;\n\
\x20   let row_base = px_shadow_pages[base];\n\
\x20
\x20   let word_index = page_x >> 5u;\n\
\x20   let bit = page_x & 31u;\n\
\x20   var rank: u32 = 0u;\n\
\x20   var word: u32 = 0u;\n\
\x20   loop {\n\
\x20       if (word > word_index) { break; }\n\
\x20       let bits = px_shadow_pages[base + 1u + word];\n\
\x20       if (word == word_index) {\n\
\x20
\x20           let mask = select((1u << (bit + 1u)) - 1u, 0xFFFFFFFFu, bit == 31u);\n\
\x20           rank = rank + countOneBits(bits & mask);\n\
\x20           break;\n\
\x20       }\n\
\x20       rank = rank + countOneBits(bits);\n\
\x20       word = word + 1u;\n\
\x20   }\n\
\x20   if (rank == 0u) { return -1; }\n\
\x20   return i32(row_base + rank - 1u);\n\
}\n\
\n\
fn px_shadow_locate(light_id: u32, level: u32, face: u32, dir: vec3<f32>, pps0: u32) -> vec3<f32> {\n\
\x20   let pps = max(pps0 >> level, 1u);\n\
\x20   let t = px_shadow_face_texel(dir, f32(pps * PX_PAGE_SIZE));\n\
\x20   let px = u32(clamp(floor(t.x / f32(PX_PAGE_SIZE)), 0.0, f32(pps) - 1.0));\n\
\x20   let py = u32(clamp(floor(t.y / f32(PX_PAGE_SIZE)), 0.0, f32(pps) - 1.0));\n\
\x20   let slot = px_shadow_page_slot(light_id, level, face, px, py);\n\
\x20   return vec3<f32>(t.x, t.y, f32(slot));\n\
}\n\
\n\
fn px_shadow_grid(level: u32) -> u32 {\n\
\x20   if (level == 1u) { return textureDimensions(point_shadow_textures_l1, 0).x / PX_PAGE_SIZE; }\n\
\x20   if (level == 2u) { return textureDimensions(point_shadow_textures_l2, 0).x / PX_PAGE_SIZE; }\n\
\x20   if (level == 3u) { return textureDimensions(point_shadow_textures_l3, 0).x / PX_PAGE_SIZE; }\n\
\x20   return textureDimensions(point_shadow_textures, 0).x / PX_PAGE_SIZE;\n\
}\n\
\n\
fn px_shadow_load(level: u32, texel: vec2<u32>, layer: i32) -> f32 {\n\
\x20   if (level == 1u) { return textureLoad(point_shadow_textures_l1, texel, layer, 0); }\n\
\x20   if (level == 2u) { return textureLoad(point_shadow_textures_l2, texel, layer, 0); }\n\
\x20   if (level == 3u) { return textureLoad(point_shadow_textures_l3, texel, layer, 0); }\n\
\x20   return textureLoad(point_shadow_textures, texel, layer, 0);\n\
}\n\
\n\
fn px_shadow_read(light_id: u32, level: u32, face: u32, slot: i32, t: vec2<f32>, depth: f32) -> f32 {\n\
\x20   let slot_u = u32(slot);\n\
\x20   let px = floor(t.x / f32(PX_PAGE_SIZE));\n\
\x20   let py = floor(t.y / f32(PX_PAGE_SIZE));\n\
\x20   let local = t - vec2<f32>(px, py) * f32(PX_PAGE_SIZE);\n\
\x20   let atlas_pages = px_shadow_grid(level);\n\
\x20   let texel = vec2<u32>(\n\
\x20       (slot_u % atlas_pages) * PX_PAGE_SIZE\n\
\x20           + u32(clamp(local.x, 0.0, f32(PX_PAGE_SIZE) - 1.0)),\n\
\x20       (slot_u / atlas_pages) * PX_PAGE_SIZE\n\
\x20           + u32(clamp(local.y, 0.0, f32(PX_PAGE_SIZE) - 1.0)),\n\
\x20   );\n\
\x20   let stored = px_shadow_load(level, texel, i32(light_id * PX_CUBE_FACES + face));\n\
\x20
\x20
\x20
\x20
\x20
\x20   return select(1.0, 0.0, depth < stored);\n\
}\n\
\n\
fn px_sample_shadow_page(light_id: u32, level: u32, dir: vec3<f32>, depth: f32) -> f32 {\n\
\x20   let off = px_shadow_light_offsets[light_id];\n\
\x20   let head = px_shadow_pages[off];\n\
\x20   let pps0 = max(head & 0xFFFFu, 1u);\n\
\x20   let atlas_pages = max(head >> 16u, 1u);\n\
\x20   let levels = max(px_shadow_pages[off + 1u] & 0xFFFFu, 1u);\n\
\x20   let face = u32(px_shadow_face_texel(dir, f32(pps0 * PX_PAGE_SIZE)).z);\n\
\x20   var lv = level;\n\
\x20   loop {\n\
\x20       if (lv >= levels) { break; }\n\
\x20       let found = px_shadow_locate(light_id, lv, face, dir, pps0);\n\
\x20       if (found.z >= 0.0) {\n\
\x20           return px_shadow_read(light_id, lv, face, i32(found.z), found.xy, depth);\n\
\x20       }\n\
\x20       lv = lv + 1u;\n\
\x20   }\n\
\x20   if (level > 0u) {\n\
\x20       lv = level;\n\
\x20       loop {\n\
\x20           lv = lv - 1u;\n\
\x20           let found = px_shadow_locate(light_id, lv, face, dir, pps0);\n\
\x20           if (found.z >= 0.0) {\n\
\x20               return px_shadow_read(light_id, lv, face, i32(found.z), found.xy, depth);\n\
\x20           }\n\
\x20           if (lv == 0u) { break; }\n\
\x20       }\n\
\x20   }\n\
\x20   return 1.0;\n\
}\n\
\n\
\n\
const PX_SHADOW_KERNEL_MAX_TEXELS: f32 = 2.0;\n\
\n\
fn px_sample_shadow_at_offset(\n\
\x20   position: vec2<f32>,\n\
\x20   coeff: f32,\n\
\x20   x_basis: vec3<f32>,\n\
\x20   y_basis: vec3<f32>,\n\
\x20   light_local: vec3<f32>,\n\
\x20   depth: f32,\n\
\x20   light_id: u32,\n\
\x20   level: u32,\n\
) -> f32 {\n\
\x20   let dir = light_local + position.x * x_basis + position.y * y_basis;\n\
\x20   return px_sample_shadow_page(light_id, level, dir, depth) * coeff;\n\
}\n\

fn fetch_point_shadow(\n\
\x20   light_id: u32,\n\
\x20   frag_position: vec4<f32>,\n\
\x20   surface_normal: vec3<f32>,\n\
\x20   frag_coord_xy: vec2<f32>,\n\
) -> f32 {\n\
\x20   let light = &clustered_lights.data[light_id];\n\
\x20   let surface_to_light = (*light).position_radius.xyz - frag_position.xyz;\n\
\x20   let surface_to_light_abs = abs(surface_to_light);\n\
\x20   let distance_to_light = max(\n\
\x20       surface_to_light_abs.x,\n\
\x20       max(surface_to_light_abs.y, surface_to_light_abs.z),\n\
\x20   );\n\
\x20
\x20
\x20
\x20
\x20
\x20
\x20   let head = px_shadow_pages[px_shadow_light_offsets[light_id]];\n\
\x20
\x20
\x20   let virtual_size = f32(head & 0xFFFFu) * f32(PX_PAGE_SIZE);\n\
\x20   let texel_world = 2.0 * distance_to_light / max(virtual_size, 1.0);\n\
\x20   let normal_offset = (*light).shadow_normal_bias * texel_world * surface_normal.xyz;\n\
\x20   let depth_offset = (*light).shadow_depth_bias * normalize(surface_to_light.xyz);\n\
\x20   let offset_position = frag_position.xyz + normal_offset + depth_offset;\n\
\x20   let frag_ls = offset_position.xyz - (*light).position_radius.xyz;\n\
\x20
\x20
\x20
\x20
\x20   let light_local = frag_ls;\n\
\x20
\x20
\x20
\x20
\x20   let face_side = f32((head & 0xFFFFu) * PX_PAGE_SIZE);\n\
\x20   let face_texel = px_shadow_face_texel(light_local, face_side);\n\
\x20   let face = u32(face_texel.z);\n\
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20   let axis = px_shadow_faces[face * 3u + 2u].xyz;\n\
\x20   let planar = dot(axis, light_local);\n\
\x20   let zw = -planar * (*light).light_custom_data.xy\n\
\x20       + (*light).light_custom_data.zw;\n\
\x20   let depth = zw.x / zw.y;\n\
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20   let light_dx = dpdx(light_local);\n\
\x20   let light_dy = dpdy(light_local);\n\
\x20   let footprint = max(length(light_dx), length(light_dy));\n\
\x20
\x20
\x20
\x20
\x20
\x20
\x20
\x20   let levels = max(px_shadow_pages[px_shadow_light_offsets[light_id] + 1u] & 0xFFFFu, 1u);\n\
\x20   let want = u32(clamp(\n\
\x20       floor(log2(max(footprint, 1e-9) / max(texel_world, 1e-9))),\n\
\x20       0.0,\n\
\x20       f32(levels - 1u),\n\
\x20   ));\n\
\x20
\x20   let texel_level = texel_world * exp2(f32(want));\n\
\x20   let kernel_texels = clamp(\n\
\x20       footprint / max(texel_level, 1e-9),\n\
\x20       1.0,\n\
\x20       PX_SHADOW_KERNEL_MAX_TEXELS,\n\
\x20   );\n\
\x20   let basis = orthonormalize(normalize(light_local))\n\
\x20       * kernel_texels * texel_level;\n\
\x20   var sum: f32 = 0.0;\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[0], PX_D3D_SAMPLE_POINT_COEFFS[0],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[1], PX_D3D_SAMPLE_POINT_COEFFS[1],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[2], PX_D3D_SAMPLE_POINT_COEFFS[2],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[3], PX_D3D_SAMPLE_POINT_COEFFS[3],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[4], PX_D3D_SAMPLE_POINT_COEFFS[4],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[5], PX_D3D_SAMPLE_POINT_COEFFS[5],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[6], PX_D3D_SAMPLE_POINT_COEFFS[6],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[7], PX_D3D_SAMPLE_POINT_COEFFS[7],\n\
\x20       basis[0], basis[1], light_local, depth, light_id, want);\n\
\x20   return sum;\n\
}\n";

pub const DEPTH_NDC_TO_VIEW_Z: &str = "fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 {\n\
                                       \x20   return -view.clip_from_view[3][2] / ndc_depth;\n\
                                       }\n";

pub fn wgpu_host_stub(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::shadows::fetch_point_shadow" => Some(POINT_SHADOW_STUB),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => Some(DEPTH_NDC_TO_VIEW_Z),
        "bevy_pbr::mesh_view_bindings::view" => Some(HOST_VIEW_STUB),
        other => bevy_stub(other),
    }
}
