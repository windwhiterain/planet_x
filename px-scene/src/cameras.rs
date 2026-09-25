use px_protocol::art::Camera;

pub const GLOBAL_DISTANCE: f32 = 3.15;
pub const CLOSE_DISTANCE: f32 = 1.40;

pub fn review() -> Vec<Camera> {
    let mut out = Vec::with_capacity(12);

    let elevation = 8.0_f32.to_radians();
    for (yaw_degrees, tag) in [
        (0.0_f32, "equator-000"),
        (90.0, "equator-090"),
        (180.0, "equator-180"),
        (270.0, "equator-270"),
    ] {
        let yaw = yaw_degrees.to_radians();
        out.push(Camera::new(
            [
                elevation.cos() * yaw.sin(),
                elevation.sin(),
                elevation.cos() * yaw.cos(),
            ],
            GLOBAL_DISTANCE,
            tag,
        ));
    }

    let half = 45.0_f32.to_radians();
    out.push(Camera::new(
        [0.0, half.sin(), half.cos()],
        GLOBAL_DISTANCE,
        "north-045",
    ));
    out.push(Camera::new(
        [0.0, -half.sin(), half.cos()],
        GLOBAL_DISTANCE,
        "south-045",
    ));

    let pole = 82.0_f32.to_radians();
    out.push(Camera::new(
        [0.0, pole.sin(), pole.cos()],
        GLOBAL_DISTANCE,
        "north-pole",
    ));
    out.push(Camera::new(
        [0.0, -pole.sin(), pole.cos()],
        GLOBAL_DISTANCE,
        "south-pole",
    ));

    for (direction, tag) in [
        ([1.0, 1.0, 1.0], "cube-corner-top"),
        ([1.0, -1.0, 1.0], "cube-corner-bottom"),
        ([1.0, 1.0, 0.0], "cube-edge-middle"),
        ([0.0, 0.0, 1.0], "cube-face-centre"),
    ] {
        out.push(Camera::new(direction, CLOSE_DISTANCE, tag));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_review_set_is_twelve_tagged_normalized_cameras() {
        let cameras = review();
        assert_eq!(cameras.len(), 12);
        let mut tags: Vec<&str> = cameras.iter().map(|camera| camera.tag.as_str()).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), 12, "tag 必须互不相同，整理对照图时要靠它定位");
        for camera in &cameras {
            let length = (camera.direction[0] * camera.direction[0]
                + camera.direction[1] * camera.direction[1]
                + camera.direction[2] * camera.direction[2])
                .sqrt();
            assert!(
                (length - 1.0).abs() < 1e-5,
                "{} 的方向没归一化：{length}",
                camera.tag
            );
            assert!(
                camera.distance > 1.0,
                "{} 的相机在球里：{}",
                camera.tag,
                camera.distance
            );
        }
    }
}
