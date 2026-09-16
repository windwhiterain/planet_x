//! 渲染目标 → 回读 → PNG。**这条路径是判据的一部分，不是一个工具函数。**
//!
//! 三条硬约束，每一条都付过代价（§104）：
//!
//! - **第 13 条：不许读交换链。** surface 只保证 `Bgra8Unorm(Srgb)` 且只保证
//!   `RENDER_ATTACHMENT` ⇒ 渲到**自己的** `Rgba8UnormSrgb`（`RENDER_ATTACHMENT | COPY_SRC`）
//!   再回读。BGRA 喂给 RGBA 的 PNG 会把红蓝换掉，而 `view_formats` 只切 sRGB、不换通道序。
//!   离线那条路根本不需要 surface。
//! - **第 3 条：PNG 必须走 `image` 的同一行代码。** Bevy 的 `save_to_disk` 做的是
//!   `try_into_dynamic()` → `to_rgb8()`（丢掉 alpha）→ `save_with_format(Png)`；
//!   实测产物是 8 位 / 颜色类型 2 / 单个 IDAT / 无辅助块。少一句就是另一份 PNG，
//!   而判据是哈希。
//! - 回读的行距必须按 `COPY_BYTES_PER_ROW_ALIGNMENT` 补齐（960 宽刚好整除，别的宽度不）。

/// 一块离屏颜色目标。判据要的每一个像素都从这里来。
pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Target {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("px_render_wgpu target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target {
            texture,
            view,
            width,
            height,
        }
    }
}

/// 判据的颜色格式，**只有这一个**：与 Bevy 的截图读回同一种。
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// 只清屏、不画东西的一条 pass。
///
/// 留成显式的一条（而不是 `LoadOp::Clear` 顺手写进别处）是因为 S0 的判据就是它：
/// 纯色能不能原样走到 PNG 上，是这条路径唯一能被单独验的机会。
pub fn clear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    color: wgpu::Color,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("px_render_wgpu clear"),
    });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("px_render_wgpu clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    queue.submit(Some(encoder.finish()));
}

/// 把目标回读成**紧凑的** RGBA8（每行 `width * 4`，没有补齐）。
pub fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
) -> Result<Vec<u8>, String> {
    let unpadded = target.width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("px_render_wgpu readback"),
        size: u64::from(padded) * u64::from(target.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("px_render_wgpu readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(target.height),
            },
        },
        wgpu::Extent3d {
            width: target.width,
            height: target.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .map_err(|err| format!("等回读超时：{err}"))?;
    receiver
        .recv()
        .map_err(|err| format!("映射没有回调：{err}"))?
        .map_err(|err| format!("映射缓冲区失败：{err}"))?;

    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((unpadded * target.height) as usize);
    for row in 0..target.height {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    staging.unmap();
    Ok(out)
}

/// 紧凑 RGBA8 → PNG。句子顺序就是 Bevy `save_to_disk` 的顺序（§104 第 3 条）。
pub fn write_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> Result<u64, String> {
    let buffer = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
        format!("回读的字节数与 {width}x{height} 对不上：这是回读错了，不是 PNG 写错了")
    })?;
    let rgb = image::DynamicImage::ImageRgba8(buffer).to_rgb8();
    rgb.save_with_format(path, image::ImageFormat::Png)
        .map_err(|err| format!("写 PNG {} 失败：{err}", path.display()))?;
    Ok(std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
}

