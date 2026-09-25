pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Target {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("px_render target"),
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

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub fn clear(device: &wgpu::Device, queue: &wgpu::Queue, target: &Target, color: wgpu::Color) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("px_render clear"),
    });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("px_render clear"),
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

pub fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
) -> Result<Vec<u8>, String> {
    let unpadded = target.width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("px_render readback"),
        size: u64::from(padded) * u64::from(target.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("px_render readback"),
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
