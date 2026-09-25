use px_field_schema::field::Field;
use px_protocol::art::Domain;

use super::palette::{
    BASIN, DUNE, FROZEN, GAS, LAND, LAVA_ROCK, MARE, Palette, REGOLITH, SHEET, WATER, mix, ramp,
};
use super::texture::{TextureData, image_from, pole_cap_filter};

fn shade(
    palette: Palette,
    height: f32,
    sea: f32,
    latitude: f32,
    wobble: f32,
) -> ([f32; 3], [f32; 3]) {
    let sea = sea.clamp(0.02, 0.98);
    let land_t = ((height - sea) / (1.0 - sea)).clamp(0.0, 1.0);
    let water_t = (height / sea).clamp(0.0, 1.0);

    match palette {
        Palette::Rocky => {
            let mut color = if height < sea {
                ramp(WATER, water_t)
            } else {
                ramp(LAND, land_t)
            };
            let cap = ((latitude - 0.80) / 0.20).clamp(0.0, 1.0);
            color = mix(color, [0.941, 0.965, 1.000], cap * 0.92);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Gas => {
            let bands = (latitude * 17.0 + wobble * 0.55).sin() * 0.5 + 0.5;
            let detail = (latitude * 47.0 + wobble * 0.90).sin() * 0.5 + 0.5;
            let t = (bands * 0.72 + detail * 0.28).clamp(0.0, 1.0);
            let mut color = ramp(GAS, t);
            let storm = ((wobble * 1.7).sin() * 0.5 + 0.5).powf(6.0);
            color = mix(color, [0.784, 0.514, 0.353], storm * 0.45);
            let pole = ((latitude - 0.74) / 0.26).clamp(0.0, 1.0);
            color = mix(color, [0.180, 0.145, 0.129], pole * 0.85);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Ice => {
            let mut color = if height < sea {
                ramp(FROZEN, water_t)
            } else {
                ramp(SHEET, land_t)
            };
            let crack = ((sea * 0.75 - height) / (sea * 0.75).max(1e-3)).clamp(0.0, 1.0);
            color = mix(color, [0.098, 0.169, 0.259], crack * 0.65);
            let cap = ((latitude - 0.62) / 0.38).clamp(0.0, 1.0);
            color = mix(color, [0.996, 1.000, 1.000], cap * 0.9);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Lava => {
            let rock = ramp(LAVA_ROCK, land_t);
            let crack = (1.0 - water_t).clamp(0.0, 1.0).powf(1.4);
            let glow = [crack * 1.000, crack * 0.330, crack * 0.060];
            let color = mix(rock, [0.996, 0.443, 0.094], crack * 0.92);
            (color, glow)
        }
        Palette::Desert => {
            let mut color = if height < sea {
                ramp(BASIN, water_t)
            } else {
                ramp(DUNE, land_t)
            };
            let strata = (height * 37.0).sin() * 0.5 + 0.5;
            color = mix(color, [0.529, 0.290, 0.196], strata * land_t * 0.16);
            let cap = ((latitude - 0.88) / 0.12).clamp(0.0, 1.0);
            color = mix(color, [0.902, 0.925, 0.941], cap * 0.45);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Moon => {
            let mut color = if height < sea {
                ramp(MARE, water_t)
            } else {
                ramp(REGOLITH, land_t)
            };
            let rays = (height * 61.0).sin() * 0.5 + 0.5;
            color = mix(color, [0.867, 0.871, 0.878], rays * land_t * 0.20);
            let cap = ((latitude - 0.86) / 0.14).clamp(0.0, 1.0);
            color = mix(color, [0.318, 0.325, 0.345], cap * 0.30);
            (color, [0.0, 0.0, 0.0])
        }
    }
}

pub(super) fn push_color(bytes: &mut Vec<u8>, color: [f32; 3]) {
    bytes.push((color[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push(255);
}

fn normalized(min: f32, max: f32, value: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        0.5
    } else {
        (value - min) / span
    }
}

pub fn texel_latitude(field: &Field, x: u32, y: u32) -> f32 {
    match field.projection {
        Domain::Equirect => {
            let v = y as f32 / (field.height.max(2) - 1) as f32;
            ((v - 0.5).abs() * 2.0).clamp(0.0, 1.0)
        }
        _ => px_protocol::art::direction_at(field.projection, field.width, field.height, x, y)[1]
            .abs(),
    }
}

pub fn surface_color(
    field: &Field,
    palette: Palette,
    sea_level: f32,
) -> (TextureData, Option<TextureData>, String) {
    let stats = field.stats();
    let mut color = Vec::with_capacity((field.width * field.height * 4) as usize);
    let mut glow = Vec::with_capacity((field.width * field.height * 4) as usize);

    for y in 0..field.height {
        for x in 0..field.width {
            let raw = field.data[(y * field.width + x) as usize];
            let height = normalized(stats.min, stats.max, raw);
            let latitude = texel_latitude(field, x, y);
            let (base, emit) = shade(palette, height, sea_level, latitude, raw * 6.283);
            push_color(&mut color, base);
            push_color(&mut glow, emit);
        }
    }

    if field.projection == Domain::Equirect {
        pole_cap_filter(&mut color, field.width, field.height);
    }
    let color_image = image_from(
        field.width,
        field.height,
        color.clone(),
        field.projection == Domain::Cube,
    );

    let texels = color.len() / 4;
    let mut sums = [0.0_f64; 3];
    for pixel in color.chunks_exact(4) {
        for channel in 0..3 {
            sums[channel] += pixel[channel] as f64;
        }
    }
    let white = color
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200)
        .count();
    let mip_levels = (field.width.max(field.height).max(1) as f32).log2().floor() as u32 + 1;
    let mip_bytes = (field.width * field.height) as f64 * 4.0 * 4.0 / 3.0;
    let audit = format!(
        "         贴图 {}{}×{}：平均 RGB ({:.0},{:.0},{:.0})，近白像素 {:.1}%，mip {mip_levels} 级（约 {:.1} MB）",
        palette.name(),
        field.width,
        field.height,
        sums[0] / texels as f64,
        sums[1] / texels as f64,
        sums[2] / texels as f64,
        white as f64 * 100.0 / texels as f64,
        mip_bytes / 1e6,
    );

    let glow_image = match palette {
        Palette::Lava => {
            if field.projection == Domain::Equirect {
                pole_cap_filter(&mut glow, field.width, field.height);
            }
            Some(image_from(
                field.width,
                field.height,
                glow,
                field.projection == Domain::Cube,
            ))
        }
        _ => None,
    };
    (color_image, glow_image, audit)
}

pub(super) fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        let payload = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7c00 | payload;
    }

    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let mut half = (mantissa >> shift) as u16;
        if (mantissa >> (shift - 1)) & 1 == 1 {
            half += 1;
        }
        return sign | half;
    }

    let mut half = ((unbiased as u32) << 10) as u16 | (mantissa >> 13) as u16;
    if mantissa & 0x1000 != 0 {
        half += 1;
    }
    sign | half
}
