use px_protocol::art::{Domain, direction_at, uv_of};

#[test]
fn every_domain_round_trips_through_its_own_uv() {
    let domains = [
        (Domain::Equirect, 96_u32, 48_u32),
        (Domain::Octahedral, 64, 64),
        (Domain::Cube, 78, 52),
        (Domain::CubeMap, 32, 192),
    ];
    for (domain, width, height) in domains {
        let mut worst = 0.0_f32;
        for y in 0..height {
            for x in 0..width {
                let direction = direction_at(domain, width, height, x, y);
                let uv = uv_of(domain, direction, width, height);
                let back = direction_at(
                    domain,
                    width,
                    height,
                    ((uv[0] * width as f32) as u32).min(width - 1),
                    ((uv[1] * height as f32) as u32).min(height - 1),
                );
                for axis in 0..3 {
                    worst = worst.max((direction[axis] - back[axis]).abs());
                }
            }
        }
        assert!(
            worst < 0.08,
            "{:?} 的 UV 往返偏差过大：{worst}",
            domain
        );
    }
}
