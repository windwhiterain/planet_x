pub struct SpanMatch {
    pub label: &'static str,
    pub bevy: Option<&'static str>,
    pub why: &'static str,
}

pub const SPAN_MATCH: &[SpanMatch] = &[
    SpanMatch {
        label: "prepass",
        bevy: Some("early prepass"),
        why: "bevy_core_pipeline-0.19.1/src/prepass/node.rs:82（early_prepass 传给 run_prepass_system 的 label）",
    },
    SpanMatch {
        label: "copy_depth",
        bevy: None,
        why: "bevy_core_pipeline-0.19.1/src/prepass/node.rs:249：Bevy 自己的深度拷贝在 pass_span.end()（:242）之后 ⇒ 两边都不在 span 里",
    },
    SpanMatch {
        label: "point_shadow",
        bevy: None,
        why: "bevy_pbr-0.19.1/src/render/light.rs:2880：只有 CPU 的 info_span!，没有 pass_span ⇒ Bevy 的 gpu_ms 不含影子图",
    },
    SpanMatch {
        label: "opaque",
        bevy: Some("main_opaque_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_opaque_pass_3d_node.rs:75",
    },
    SpanMatch {
        label: "sky",
        bevy: Some("main_opaque_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_opaque_pass_3d_node.rs:99-107：天空盒**画在同一条 render pass 里**",
    },
    SpanMatch {
        label: "transparent",
        bevy: Some("main_transparent_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_transparent_pass_3d_node.rs:87",
    },
    SpanMatch {
        label: "blit",
        bevy: Some("upscaling"),
        why: "bevy_core_pipeline-0.19.1/src/upscaling/node.rs:86（time_span，编码器级）",
    },
];

pub const BEVY_ONLY: &[(&str, &str)] = &[
    (
        "bin_unpacking",
        "bevy_pbr-0.19.1/src/render/gpu_preprocess.rs:545（compute）",
    ),
    (
        "early_mesh_preprocessing",
        "bevy_pbr-0.19.1/src/render/gpu_preprocess.rs:630（compute）",
    ),
    (
        "clustering",
        "bevy_pbr-0.19.1/src/cluster/gpu.rs:872（编码器级 time_span）",
    ),
];

pub fn bevy_span_of(label: &str) -> Result<Option<&'static str>, String> {
    if let Some(entry) = SPAN_MATCH.iter().find(|entry| entry.label == label) {
        return Ok(entry.bevy);
    }
    if let Some(family) = SPAN_FAMILIES.iter().find(|family| (family.matches)(label)) {
        return Ok(family.bevy);
    }
    Err(format!(
        "pass '{label}' 不在这张对照表里（表里只有：{}；另有一族 {}）—— \
         匹配子集因此算不出来，而**算错的子集比没有子集更坏**。\
         要加一条，得先在 `spans.rs::SPAN_MATCH` 里指得出 Bevy 那一侧的出处",
        SPAN_MATCH
            .iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>()
            .join(" / "),
        SPAN_FAMILIES
            .iter()
            .map(|family| family.shape)
            .collect::<Vec<_>>()
            .join(" / ")
    ))
}

pub struct SpanFamily {
    pub shape: &'static str,
    pub bevy: Option<&'static str>,
    pub why: &'static str,
    pub matches: fn(&str) -> bool,
}

pub const SPAN_FAMILIES: &[SpanFamily] = &[SpanFamily {
    shape: "point_shadow_{灯号}_{面}",
    bevy: None,
    why: "bevy_pbr-0.19.1/src/render/light.rs:2880（同 `point_shadow`：只有 CPU info_span!）",
    matches: is_point_shadow_face,
}];

fn is_point_shadow_face(label: &str) -> bool {
    let Some(rest) = label.strip_prefix("point_shadow_") else {
        return false;
    };
    let Some((light, face)) = rest.split_once('_') else {
        return false;
    };
    !light.is_empty()
        && light.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(face, "+x" | "-x" | "+y" | "-y" | "+z" | "-z")
}

pub struct FrameStamps<'a> {
    query_set: &'a wgpu::QuerySet,
    layout: &'a [px_pass::PassSlots],
    frame: (u32, u32),
    resolve: &'a wgpu::Buffer,
    resolve_offset: u64,
}

impl<'a> FrameStamps<'a> {
    pub fn stride(&self) -> u32 {
        self.frame.1 + 1
    }

    pub fn stride_of(passes: u32) -> u32 {
        px_pass::TIMESTAMP_SLOTS_PER_PASS * passes + px_pass::TIMESTAMP_FRAME_SLOTS
    }

    pub fn passes(&self) -> u32 {
        self.layout.len() as u32
    }

    pub fn begin_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(self.query_set, self.frame.0);
    }

    pub fn end_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(self.query_set, self.frame.1);
    }

    pub fn resolve_now(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(
            self.query_set,
            0..self.stride(),
            self.resolve,
            self.resolve_offset,
        );
    }

    pub fn executor_view(&self) -> px_pass::PassTimestamps<'_> {
        px_pass::PassTimestamps::new(self.query_set, self.layout)
    }

    pub fn pass_slots(&self, index: usize) -> Option<px_pass::PassSlots> {
        self.layout.get(index).copied()
    }
}

pub fn missing_features(adapter: &wgpu::Adapter) -> Vec<&'static str> {
    let features = adapter.features();
    let mut missing = Vec::new();
    for (feature, name) in [
        (wgpu::Features::TIMESTAMP_QUERY, "TIMESTAMP_QUERY"),
        (
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            "TIMESTAMP_QUERY_INSIDE_ENCODERS",
        ),
        (
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
            "TIMESTAMP_QUERY_INSIDE_PASSES",
        ),
    ] {
        if !features.contains(feature) {
            missing.push(name);
        }
    }
    missing
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub p50: f64,
    pub min: f64,
    pub max: f64,
}

impl Stats {
    fn of(mut values: Vec<f64>) -> Stats {
        values.sort_by(|one, two| one.partial_cmp(two).expect("毫秒不会是 NaN"));
        let mid = values.len() / 2;
        Stats {
            p50: values[mid],
            min: values[0],
            max: values[values.len() - 1],
        }
    }
}

pub struct PassReading {
    pub index: usize,
    pub label: String,
    pub kind: &'static str,
    pub bevy: Option<&'static str>,
    pub envelope: Stats,
    pub inside: Option<Stats>,
}

pub struct Reading {
    pub block: u32,
    pub warm: u32,
    pub frames: u32,
    pub passes: Vec<PassReading>,
    pub matched: Stats,
    pub inside_all: Stats,
    pub envelope_all: Stats,
    pub frame: Stats,
    pub timestamp_calls: u32,
}

impl Reading {
    pub fn report(&self, scene: &str, width: u32, height: u32) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in SPAN_MATCH {
            lines.push(format!(
                "[span-map] pass={} bevy={} why={}",
                entry.label,
                entry.bevy.unwrap_or("none"),
                entry.why.replace(' ', "_")
            ));
        }
        for (name, why) in BEVY_ONLY {
            lines.push(format!(
                "[span-map] pass=none bevy={name} why={}",
                why.replace(' ', "_")
            ));
        }
        for family in SPAN_FAMILIES {
            lines.push(format!(
                "[span-map] pass={} bevy={} why={}",
                family.shape,
                family.bevy.unwrap_or("none"),
                family.why.replace(' ', "_")
            ));
        }
        for pass in &self.passes {
            let mut line = format!(
                "[span] scene={scene} size={width}x{height} block={} warm={} frames={} pass={}:{} kind={} bevy={} envelope_p50_ms={:.4} envelope_min_ms={:.4} envelope_max_ms={:.4}",
                self.block,
                self.warm,
                self.frames,
                pass.index,
                pass.label,
                pass.kind,
                pass.bevy.unwrap_or("none"),
                pass.envelope.p50,
                pass.envelope.min,
                pass.envelope.max
            );
            match &pass.inside {
                Some(inside) => line.push_str(&format!(
                    " inside_p50_ms={:.4} inside_min_ms={:.4} inside_max_ms={:.4}",
                    inside.p50, inside.min, inside.max
                )),
                None => line.push_str(" inside_p50_ms=none(copy：不开 render pass)"),
            }
            lines.push(line);
        }
        lines.push(format!(
            "[span-sum] scene={scene} size={width}x{height} block={} warm={} frames={} matched_inside_p50_ms={:.4} matched_inside_min_ms={:.4} matched_inside_max_ms={:.4}",
            self.block, self.warm, self.frames, self.matched.p50, self.matched.min, self.matched.max
        ));
        lines.push(format!(
            "[span-sum] scene={scene} block={} inside_all_p50_ms={:.4} envelope_all_p50_ms={:.4} frame_p50_ms={:.4} frame_min_ms={:.4} frame_max_ms={:.4} timestamp_calls_per_frame={}",
            self.block, self.inside_all.p50, self.envelope_all.p50, self.frame.p50, self.frame.min, self.frame.max, self.timestamp_calls
        ));
        lines
    }
}

fn region_bytes(stride: u32) -> u64 {
    let raw = u64::from(stride) * u64::from(wgpu::QUERY_SIZE);
    let align = u64::from(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
    raw.div_ceil(align) * align
}

pub struct Recorder {
    query_set: wgpu::QuerySet,
    layout: Vec<px_pass::PassSlots>,
    frame: (u32, u32),
    resolve: wgpu::Buffer,
    buffer: wgpu::Buffer,
    bytes: u64,
    region_ticks: u32,
    frames: u32,
    warm: u32,
    measured: u32,
    labels: Vec<(String, px_pass::PassKind)>,
    period_ns: f32,
    calls: Vec<u32>,
}

impl Recorder {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        labels: Vec<(String, px_pass::PassKind)>,
        blocks: u32,
        warm: u32,
        measured: u32,
    ) -> Result<Recorder, String> {
        if labels.is_empty() {
            return Err("这份文档一条 pass 都没有：没有 span 可量".to_string());
        }
        if measured == 0 {
            return Err("测量帧是 0 帧：那一位必须是正的（预热帧不算数）".to_string());
        }
        if blocks == 0 {
            return Err("轮数是 0：那一位必须是正的".to_string());
        }
        let kinds: Vec<px_pass::PassKind> = labels.iter().map(|(_, kind)| *kind).collect();
        let (layout, frame_begin) = px_pass::frame_slots(&kinds);
        let stride = frame_begin + px_pass::TIMESTAMP_FRAME_SLOTS;
        if stride > wgpu::QUERY_SET_MAX_QUERIES {
            return Err(format!(
                "一帧 {stride} 格，超过一台设备最多能有的 {} 格（wgpu 的 QUERY_SET_MAX_QUERIES）：\
                 这份文档有 {} 条 pass",
                wgpu::QUERY_SET_MAX_QUERIES,
                labels.len()
            ));
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("px_render 逐条 pass 的时间戳（一帧那几格）"),
            ty: wgpu::QueryType::Timestamp,
            count: stride,
        });
        let region = region_bytes(stride);
        let frames = blocks
            .checked_mul(warm + measured)
            .ok_or_else(|| format!("{blocks} 轮 × {} 帧：帧数溢出了", warm + measured))?;
        let bytes = region * u64::from(frames);
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_render 时间戳 resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_render 时间戳回读"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Recorder {
            query_set,
            layout,
            frame: (frame_begin, frame_begin + 1),
            resolve,
            buffer,
            bytes,
            region_ticks: (region / u64::from(wgpu::QUERY_SIZE)) as u32,
            frames,
            warm,
            measured,
            labels,
            period_ns: queue.get_timestamp_period(),
            calls: Vec::with_capacity(frames as usize),
        })
    }

    pub fn stamps(&self, frame: u32) -> FrameStamps<'_> {
        FrameStamps {
            query_set: &self.query_set,
            layout: &self.layout,
            frame: self.frame,
            resolve: &self.resolve,
            resolve_offset: u64::from(frame)
                * u64::from(self.region_ticks)
                * u64::from(wgpu::QUERY_SIZE),
        }
    }

    pub fn passes(&self) -> u32 {
        self.layout.len() as u32
    }

    pub fn note_calls(&mut self, calls: u32) {
        self.calls.push(calls);
    }

    pub fn finish(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<Reading>, String> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_render 时间戳回读"),
        });
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.buffer, 0, Some(self.bytes));
        queue.submit(Some(encoder.finish()));

        let slice = self.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(120)),
            })
            .map_err(|err| format!("等时间戳回读超时：{err}"))?;
        receiver
            .recv()
            .map_err(|err| format!("时间戳映射没有回调：{err}"))?
            .map_err(|err| format!("时间戳缓冲区映射失败：{err}"))?;

        let ticks: Vec<u64> = {
            let data = slice.get_mapped_range();
            data.chunks_exact(wgpu::QUERY_SIZE as usize)
                .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 字节一格")))
                .collect()
        };
        let _ = self.buffer.unmap();

        if self.calls.len() != self.frames as usize {
            return Err(format!(
                "记下来的是 {} 帧、而这一套槽排的是 {} 帧：帧数与槽对不上，读数不可信",
                self.calls.len(),
                self.frames
            ));
        }
        let calls = self.calls[0];
        if let Some(other) = self.calls.iter().find(|value| **value != calls) {
            return Err(format!(
                "每一帧该发同样多条时间戳，实测 {} 与 {calls} 两种 —— \
                 条数一变，说明这两帧跑的不是同一条路，读数不可比",
                other
            ));
        }

        let to_ms = |begin: u64, end: u64| -> f64 {
            end.wrapping_sub(begin) as f64 * f64::from(self.period_ns) / 1e6
        };

        let layout = self.stamps(0);
        let region = self.region_ticks;
        let at = |frame: u32, slot: u32| -> u64 { ticks[(frame * region + slot) as usize] };
        let per_block = self.warm + self.measured;

        let mut out = Vec::with_capacity((self.frames / per_block) as usize);
        for block in 0..(self.frames / per_block) {
            let measured: Vec<u32> = (0..self.measured)
                .map(|index| block * per_block + self.warm + index)
                .collect();

            let mut passes = Vec::with_capacity(self.labels.len());
            for (index, (label, kind)) in self.labels.iter().enumerate() {
                let slots = layout
                    .pass_slots(index)
                    .ok_or_else(|| format!("第 {index} 条 pass '{label}' 的格不在排布里"))?;
                let mut envelope = Vec::with_capacity(measured.len());
                let mut inside = Vec::with_capacity(measured.len());
                for frame in &measured {
                    envelope.push(to_ms(
                        at(*frame, slots.envelope.0),
                        at(*frame, slots.envelope.1),
                    ));
                    if let Some((begin, end)) = slots.inside {
                        inside.push(to_ms(at(*frame, begin), at(*frame, end)));
                    }
                }
                let bevy = bevy_span_of(label)?;
                passes.push(PassReading {
                    index,
                    label: label.clone(),
                    kind: kind.name(),
                    bevy,
                    envelope: Stats::of(envelope),
                    inside: if inside.is_empty() {
                        None
                    } else {
                        Some(Stats::of(inside))
                    },
                });
            }

            let mut matched = Vec::with_capacity(measured.len());
            let mut inside_all = Vec::with_capacity(measured.len());
            let mut envelope_all = Vec::with_capacity(measured.len());
            let mut frame = Vec::with_capacity(measured.len());
            for at_frame in &measured {
                let mut sum_matched = 0.0;
                let mut sum_inside = 0.0;
                let mut sum_envelope = 0.0;
                for (index, (_, _)) in self.labels.iter().enumerate() {
                    let slots = layout
                        .pass_slots(index)
                        .ok_or_else(|| format!("第 {index} 条 pass 的格不在排布里"))?;
                    sum_envelope += to_ms(
                        at(*at_frame, slots.envelope.0),
                        at(*at_frame, slots.envelope.1),
                    );
                    if let Some((begin, end)) = slots.inside {
                        let value = to_ms(at(*at_frame, begin), at(*at_frame, end));
                        sum_inside += value;
                        if passes[index].bevy.is_some() {
                            sum_matched += value;
                        }
                    }
                }
                let (frame_begin, frame_end) = layout.frame;
                matched.push(sum_matched);
                inside_all.push(sum_inside);
                envelope_all.push(sum_envelope);
                frame.push(to_ms(at(*at_frame, frame_begin), at(*at_frame, frame_end)));
            }

            out.push(Reading {
                block,
                warm: self.warm,
                frames: self.measured,
                passes,
                matched: Stats::of(matched),
                inside_all: Stats::of(inside_all),
                envelope_all: Stats::of(envelope_all),
                frame: Stats::of(frame),
                timestamp_calls: calls,
            });
        }
        Ok(out)
    }
}
