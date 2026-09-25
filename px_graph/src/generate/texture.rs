use px_field_schema::field::Field;
use px_protocol::art::{CUBE_COLUMNS, CUBE_FACES, Domain, TextureFormat};

use super::shade::{half_from_f32, push_color};

pub use px_protocol::art::TextureData;

fn mip_chain_cube(width: u32, height: u32, base: &[u8]) -> (Vec<u8>, u32) {
    let mut chain = base.to_vec();
    let mut levels = 1_u32;
    let mut source = base.to_vec();
    let (mut w, mut h) = (width.max(1), height.max(1));

    while w > 1 && h > 1 {
        let next_w = (w / 2).max(1);
        let next_h = (h / 2).max(1);
        let cell = (w / CUBE_COLUMNS).max(1);
        let next_cell = (next_w / CUBE_COLUMNS).max(1);
        let mut next = vec![0_u8; (next_w * next_h * 4) as usize];

        for face in 0..CUBE_FACES {
            let column = face % CUBE_COLUMNS;
            let row = face / CUBE_COLUMNS;
            let left = column * cell;
            let top = row * cell;
            for y in 0..next_cell {
                for x in 0..next_cell {
                    let sample_x = left + (x * 2).min(cell - 1);
                    let sample_y = top + (y * 2).min(cell - 1);
                    let sample_x1 = (sample_x + 1).min(left + cell - 1);
                    let sample_y1 = (sample_y + 1).min(top + cell - 1);
                    let mut sums = [0_u32; 4];
                    for (ax, ay) in [
                        (sample_x, sample_y),
                        (sample_x1, sample_y),
                        (sample_x, sample_y1),
                        (sample_x1, sample_y1),
                    ] {
                        let index = (ay * w + ax) as usize * 4;
                        for channel in 0..4 {
                            sums[channel] += source[index + channel] as u32;
                        }
                    }
                    let dx = column * next_cell + x;
                    let dy = row * next_cell + y;
                    if dx < next_w && dy < next_h {
                        let out = (dy * next_w + dx) as usize * 4;
                        for channel in 0..4 {
                            next[out + channel] = (sums[channel] / 4) as u8;
                        }
                    }
                }
            }
        }

        chain.extend_from_slice(&next);
        source = next;
        w = next_w;
        h = next_h;
        levels += 1;
    }

    (chain, levels)
}

fn mip_chain(width: u32, height: u32, base: &[u8]) -> (Vec<u8>, u32) {
    let mut chain = base.to_vec();
    let mut levels = 1_u32;
    let mut source = base.to_vec();
    let (mut w, mut h) = (width.max(1), height.max(1));

    while w > 1 || h > 1 {
        let next_w = (w / 2).max(1);
        let next_h = (h / 2).max(1);
        let mut next = vec![0_u8; (next_w * next_h * 4) as usize];

        for y in 0..next_h {
            for x in 0..next_w {
                let mut sums = [0_u32; 4];
                for step_y in 0..2 {
                    for step_x in 0..2 {
                        let sample_x = ((x * 2 + step_x) % w) as usize;
                        let sample_y = ((y * 2 + step_y).min(h - 1)) as usize;
                        let index = (sample_y * w as usize + sample_x) * 4;
                        for channel in 0..4 {
                            sums[channel] += source[index + channel] as u32;
                        }
                    }
                }
                let out = ((y * next_w + x) * 4) as usize;
                for channel in 0..4 {
                    next[out + channel] = (sums[channel] / 4) as u8;
                }
            }
        }

        chain.extend_from_slice(&next);
        source = next;
        w = next_w;
        h = next_h;
        levels += 1;
    }

    (chain, levels)
}

fn pole_cap_rows(height: u32) -> u32 {
    (height / 32).max(2)
}

pub(super) fn pole_cap_filter(pixels: &mut [u8], width: u32, height: u32) {
    let rows = pole_cap_rows(height);
    for offset in 0..rows {
        let weight = 1.0 - offset as f32 / rows as f32;
        for row in [offset, height - 1 - offset] {
            let start = (row * width) as usize * 4;
            let end = start + width as usize * 4;
            let Some(slice) = pixels.get_mut(start..end) else {
                continue;
            };
            let mut mean = [0.0_f32; 3];
            for pixel in slice.chunks_exact(4) {
                for channel in 0..3 {
                    mean[channel] += pixel[channel] as f32;
                }
            }
            for value in mean.iter_mut() {
                *value /= width as f32;
            }
            for pixel in slice.chunks_exact_mut(4) {
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as f32 * (1.0 - weight)
                        + mean[channel] * weight)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

pub(super) fn image_from(width: u32, height: u32, data: Vec<u8>, cube: bool) -> TextureData {
    let (chain, levels) = if cube {
        mip_chain_cube(width, height, &data)
    } else {
        mip_chain(width, height, &data)
    };
    TextureData::new(width, height, 1, levels, TextureFormat::Rgba8Srgb, chain)
}

pub fn coverage_cube(mask: &Field, slopes: [&Field; 3]) -> Result<TextureData, String> {
    let field = mask;
    if field.projection != Domain::CubeMap {
        return Err(format!(
            "云覆盖度需要 CubeMap 产物，这份是 {:?}",
            field.projection
        ));
    }
    let face = field.width.max(1);
    if field.height != face * CUBE_FACES {
        return Err(format!(
            "云覆盖度的行数应当是 {face} × {CUBE_FACES} = {}，实际 {}",
            face * CUBE_FACES,
            field.height
        ));
    }
    for slope in slopes {
        if slope.projection != Domain::CubeMap
            || slope.width != face
            || slope.height != field.height
        {
            return Err(format!(
                "云的梯度场必须和覆盖度同形（{face}×{}），这份是 {:?} {}×{}",
                field.height, slope.projection, slope.width, slope.height
            ));
        }
    }

    let mut bytes = Vec::with_capacity(field.data.len() * 8);
    for index in 0..field.data.len() {
        for value in [
            field.data[index],
            slopes[0].data[index],
            slopes[1].data[index],
            slopes[2].data[index],
        ] {
            bytes.extend_from_slice(&half_from_f32(value).to_le_bytes());
        }
    }

    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

pub fn color_cube(red: &Field, green: &Field, blue: &Field) -> Result<TextureData, String> {
    let face = red.width.max(1);
    for (name, field) in [("R", red), ("G", green), ("B", blue)] {
        if field.projection != Domain::CubeMap {
            return Err(format!(
                "颜色立方贴图的 {name} 通道需要 CubeMap 产物，这份是 {:?}",
                field.projection
            ));
        }
        if field.width != face || field.height != face * CUBE_FACES {
            return Err(format!(
                "颜色立方贴图的 {name} 通道形状应当是 {face}×{}（`width × 6`），实际 {}×{}",
                face * CUBE_FACES,
                field.width,
                field.height,
            ));
        }
    }

    let mut bytes = Vec::with_capacity(red.data.len() * 8);
    for index in 0..red.data.len() {
        for value in [red.data[index], green.data[index], blue.data[index]] {
            bytes.extend_from_slice(&half_from_f32(value).to_le_bytes());
        }
        bytes.extend_from_slice(&half_from_f32(1.0).to_le_bytes());
    }

    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

pub fn field_cube(field: &Field) -> Result<TextureData, String> {
    if field.projection != Domain::CubeMap {
        return Err(format!(
            "场当立方贴图需要 CubeMap 产物，这份是 {:?}",
            field.projection
        ));
    }
    let face = field.width.max(1);
    if field.height != face * CUBE_FACES {
        return Err(format!(
            "场的行数应当是 {face} × {CUBE_FACES} = {}，实际 {}",
            face * CUBE_FACES,
            field.height
        ));
    }

    let mut bytes = Vec::with_capacity(field.data.len() * 8);
    for value in &field.data {
        bytes.extend_from_slice(&half_from_f32(*value).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&half_from_f32(1.0).to_le_bytes());
    }

    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

pub fn stars(face: u32) -> TextureData {
    let size = face.max(4);
    let mut data = Vec::with_capacity((size * size * 6 * 4) as usize);

    for face_index in 0..CUBE_FACES {
        for y in 0..size {
            for x in 0..size {
                let s = (x as f32 + 0.5) / size as f32;
                let t = (y as f32 + 0.5) / size as f32;
                let direction = px_protocol::art::cube_direction(face_index, s, t);
                let mut hash = (face_index as u64)
                    .wrapping_mul(0x9e37_79b9)
                    .wrapping_add((x as u64).wrapping_mul(0x85eb_ca6b))
                    .wrapping_mul(0xc2b2_ae35)
                    .wrapping_add((y as u64).wrapping_mul(0x27d4_eb2f));
                hash ^= hash >> 15;
                hash = hash.wrapping_mul(0x2545_f491);
                hash ^= hash >> 13;
                let value = (hash & 0xffff) as f32 / 65535.0;
                let brightness = if value > 0.99900 {
                    1.0
                } else if value > 0.99750 {
                    0.55
                } else if value > 0.99550 {
                    0.22
                } else {
                    0.0
                };
                let blue = 0.86 + 0.14 * ((hash >> 16) as f32 / 65535.0);
                let warp = 0.90 + 0.10 * (direction[1] * 0.5 + 0.5);
                let level = (brightness * 255.0 * warp) as u8;
                data.push(level);
                data.push(level);
                data.push((level as f32 * blue) as u8);
                data.push(255);
            }
        }
    }

    TextureData::new(size, size, CUBE_FACES, 1, TextureFormat::Rgba8Srgb, data)
}

pub fn ring_band(width: u32, height: u32) -> TextureData {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for _row in 0..height {
        for x in 0..width {
            let t = x as f32 / (width.max(2) - 1) as f32;
            let mut density = 0.34 + 0.32 * (t * 47.0).sin().abs();
            density *= 0.58 + 0.42 * (t * 13.0 + 0.7).sin().abs();
            density = (density + 0.18 * ((t * 211.0).sin() * 0.5 + 0.5)).clamp(0.0, 1.0);
            if (t - 0.635).abs() < 0.030 {
                density *= 0.08;
            }
            if (t - 0.340).abs() < 0.012 {
                density *= 0.34;
            }
            let edge = (t / 0.07).clamp(0.0, 1.0) * ((1.0 - t) / 0.10).clamp(0.0, 1.0);
            let alpha = (density * edge).clamp(0.0, 1.0);
            let shade = 0.70 + 0.30 * (0.5 + 0.5 * (t * 61.0).sin());
            push_color(&mut data, [0.878 * shade, 0.827 * shade, 0.729 * shade]);
            let last = data.len() - 1;
            data[last] = (alpha * 240.0) as u8;
        }
    }
    image_from(width, height, data, false)
}

pub(super) fn flatten2(values: &[[f32; 2]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for value in values {
        out.extend_from_slice(value);
    }
    out
}
