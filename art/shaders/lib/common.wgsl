#define_import_path planet_x::common

// ⚠ 太阳方向**不许写死**：只能从场景那盏灯里读（`planet_x::light::sun_light`）——
// 常量与灯是两个真相，改灯不动常量的时候，云和云影跟着常量走、山的影子跟着灯走，
// 差一点点最难查。

struct ShellHit {
    entry: f32,
    exit: f32,
    valid: bool,
};

fn sphere_entry(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> f32 {
    let along = dot(origin, direction);
    let disc = along * along - (dot(origin, origin) - radius * radius);
    if disc < 0.0 {
        return -1.0;
    }
    return -along - sqrt(disc);
}

fn sphere_exit(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> f32 {
    let along = dot(origin, direction);
    let disc = along * along - (dot(origin, origin) - radius * radius);
    if disc < 0.0 {
        return -1.0;
    }
    return -along + sqrt(disc);
}

fn shell_thickness(
    origin: vec3<f32>,
    direction: vec3<f32>,
    inner: f32,
    outer: f32,
) -> ShellHit {
    let outer_entry = sphere_entry(origin, direction, outer);
    let outer_exit = sphere_exit(origin, direction, outer);
    if outer_exit < 0.0 || outer_entry > outer_exit {
        return ShellHit(0.0, 0.0, false);
    }

    let start = max(outer_entry, 0.0);
    var end = outer_exit;

    let inner_entry = sphere_entry(origin, direction, inner);
    if inner_entry > 0.0 {
        end = min(end, inner_entry);
    }

    if end <= start {
        return ShellHit(start, start, false);
    }
    return ShellHit(start, end, true);
}

fn density_profile(altitude: f32, scale_height: f32) -> f32 {
    return exp(-max(altitude, 0.0) / max(scale_height, 1e-4));
}
