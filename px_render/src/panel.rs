use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};

use crate::cook::{self, Cook};
use crate::edit::{Kind, ParamStore, Scalar};

pub struct Panel {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: Option<Renderer>,
    cook: Cook,
    editor: Option<ParamStore>,
    recipe: Option<String>,
    notice: String,
    open: bool,
}

enum Action {
    Set {
        node: usize,
        field: usize,
        value: Scalar,
    },
    Reset {
        node: usize,
    },
    Cook,
    SaveToArt,
    Reload,
}

impl Panel {
    pub fn new(
        window: &winit::window::Window,
        recipe: Option<String>,
        editor: Option<ParamStore>,
    ) -> Panel {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        install_cjk_font(&ctx);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let notice = match (&recipe, &editor) {
            (_, Some(store)) => format!(
                "会话副本 {}（art/ 要按 Save 才动）",
                store.store_root().display()
            ),
            (Some(recipe), None) => {
                format!("场景配方 `{recipe}` 开不起来（理由见窗口日志）⇒ 面板这一段没有控件")
            }
            (None, None) => {
                "没有可编辑的图：起窗口时给 `--edit <场景配方名>`（例如 `--edit orbit-bare`）"
                    .to_string()
            }
        };
        Panel {
            ctx,
            state,
            renderer: None,
            cook: Cook::new(),
            editor,
            recipe,
            notice,
            open: true,
        }
    }

    pub fn ready(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.renderer.is_some() {
            return;
        }
        self.state
            .set_max_texture_side(device.limits().max_texture_dimension_2d as usize);
        self.renderer = Some(Renderer::new(
            device,
            format,
            RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                ..RendererOptions::default()
            },
        ));
    }

    pub fn wants_pointer(&self) -> bool {
        self.open && self.ctx.egui_wants_pointer_input()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub fn poll(&mut self) -> Option<cook::SceneUpdate> {
        self.cook.pump()
    }

    pub fn busy(&self) -> bool {
        self.cook.busy()
    }

    pub fn paint(
        &mut self,
        window: &winit::window::Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: [u32; 2],
        root: &std::path::Path,
    ) {
        if self.renderer.is_none() {
            return;
        }
        let input = self.state.take_egui_input(window);
        let Panel {
            ctx,
            state,
            cook,
            editor,
            recipe,
            notice,
            ..
        } = self;
        let mut render = Render {
            cook,
            editor,
            recipe,
            notice,
        };
        let full = ctx.run_ui(input, |ui| render.ui(ui, root));
        state.handle_platform_output(window, full.platform_output.clone());
        let jobs = ctx.tessellate(full.shapes, full.pixels_per_point);
        let descriptor = ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: full.pixels_per_point,
        };

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        for (id, delta) in &full.textures_delta.set {
            renderer.update_texture(device, queue, *id, delta);
        }
        let user = renderer.update_buffers(device, queue, encoder, &jobs, &descriptor);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("px_render 面板"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
        queue.submit(user);
        for id in &full.textures_delta.free {
            renderer.free_texture(id);
        }
    }
}

struct Render<'a> {
    cook: &'a mut Cook,
    editor: &'a mut Option<ParamStore>,
    recipe: &'a Option<String>,
    notice: &'a mut String,
}

impl Render<'_> {
    fn ui(&mut self, ui: &mut egui::Ui, root: &std::path::Path) {
        let mut actions: Vec<Action> = Vec::new();
        let available = ui.available_width();
        let cap = (available * 0.44).clamp(260.0, 460.0);
        let default = (available * 0.38).clamp(240.0, 400.0);
        egui::Panel::left("px_edit")
            .resizable(true)
            .show_separator_line(false)
            .default_size(default)
            .size_range(240.0..=cap)
            .show(ui, |ui| {
                ui.set_clip_rect(ui.max_rect());
                self.header(ui);
                self.toolbar(ui, &mut actions);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("px_edit_nodes")
                    .max_height(ui.available_height() - 160.0)
                    .show(ui, |ui| self.nodes(ui, &mut actions));
                ui.separator();
                self.log(ui);
            });
        for action in actions {
            self.perform(action, root);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("调参面板");
            match self.recipe {
                Some(recipe) => ui.label(format!("场景配方 `{recipe}`")),
                None => ui.label("（没开编辑：--edit <场景配方名>）"),
            };
        });
        ui.label(&*self.notice);
        if let Some(store) = &self.editor {
            let dirty = store.nodes().iter().filter(|node| node.dirty()).count();
            ui.label(format!(
                "{} 张图｜{} 个节点｜{dirty} 个改过",
                store.cook_order().len(),
                store.nodes().len(),
            ));
        }
        ui.label(format!("烘图：{}", self.cook.status()));
        if let Some(failure) = self.cook.failure() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 120),
                format!("上一次失败：{failure}"),
            );
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let busy = self.cook.busy();
        let has_editor = self.editor.is_some();
        let dirty = self.editor.as_ref().is_some_and(ParamStore::dirty);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(has_editor && !busy, egui::Button::new("Cook 烘"))
                .on_hover_text("把会话副本里的参数按顺序烘成新的场景产物，窗口随即重载")
                .clicked()
            {
                actions.push(Action::Cook);
            }
            if ui
                .add_enabled(dirty && !busy, egui::Button::new("Save art/"))
                .on_hover_text("把改过的节点写回工作树里的 art/（作品的源码）")
                .clicked()
            {
                actions.push(Action::SaveToArt);
            }
            if ui
                .add_enabled(has_editor && !busy, egui::Button::new("重读"))
                .on_hover_text("丢掉内存里那一份，重新从盘上读会话副本")
                .clicked()
            {
                actions.push(Action::Reload);
            }
        });
    }

    fn nodes(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(store) = &self.editor else {
            ui.label(
                "没有可编辑的图。开窗口时给 `--edit <场景配方名>`（例如 `--edit orbit-bare`），\
                 面板会把那份配方引用到的图读进来。",
            );
            return;
        };
        let mut last_graph = String::new();
        for (node_index, node) in store.nodes().iter().enumerate() {
            if node.graph != last_graph {
                last_graph = node.graph.clone();
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("图 {}", node.graph)).strong());
            }
            let title = if node.dirty() {
                format!("{}（改过）", node.node)
            } else {
                node.node.clone()
            };
            egui::CollapsingHeader::new(title)
                .id_salt(format!("{}::{}", node.graph, node.node))
                .default_open(node.dirty())
                .show(ui, |ui| {
                    if node.fields.is_empty() {
                        ui.label("（这个文件里没有可编辑的标量）");
                    }
                    for (field_index, field) in node.fields.iter().enumerate() {
                        let mut value = field.value.clone();
                        let changed = match (&mut value, field.kind) {
                            (Scalar::Int(number), Kind::Number) => {
                                let mut shown = *number as f64;
                                let response = match field.range {
                                    Some((min, max)) => ui.add(
                                        egui::Slider::new(&mut shown, min..=max)
                                            .text(&field.key)
                                            .step_by(step_of(min, max)),
                                    ),
                                    None => ui.add(
                                        egui::DragValue::new(&mut shown)
                                            .prefix(format!("{} = ", field.key)),
                                    ),
                                };
                                if response.changed() {
                                    *number = shown.round() as i64;
                                    true
                                } else {
                                    false
                                }
                            }
                            (Scalar::Float(number), Kind::Number) => {
                                let response = match field.range {
                                    Some((min, max)) => ui.add(
                                        egui::Slider::new(number, min..=max)
                                            .text(&field.key)
                                            .step_by(step_of(min, max)),
                                    ),
                                    None => ui.add(
                                        egui::DragValue::new(number)
                                            .prefix(format!("{} = ", field.key)),
                                    ),
                                };
                                response.changed()
                            }
                            (Scalar::Bool(flag), Kind::Bool) => {
                                ui.checkbox(flag, &field.key).changed()
                            }
                            (Scalar::Text(text), Kind::Text) => {
                                ui.horizontal(|ui| {
                                    ui.label(&field.key);
                                    ui.text_edit_singleline(text).changed()
                                })
                                .inner
                            }
                            _ => {
                                ui.colored_label(
                                    egui::Color32::from_rgb(255, 160, 80),
                                    format!("{}：面板记的类型与值对不上", field.key),
                                );
                                false
                            }
                        };
                        if field.dirty() {
                            ui.label(
                                egui::RichText::new(format!("（原值 {}）", field.original.label()))
                                    .weak(),
                            );
                        }
                        if changed {
                            actions.push(Action::Set {
                                node: node_index,
                                field: field_index,
                                value,
                            });
                        }
                    }
                    if node.dirty() {
                        ui.horizontal(|ui| {
                            if ui.button("复位（从 art/ 读回）").clicked() {
                                actions.push(Action::Reset { node: node_index });
                            }
                        });
                    }
                });
        }
    }

    fn log(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("烘图日志")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("px_edit_log")
                    .max_height(120.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let lines: Vec<&str> = self.cook.tail(120).collect();
                        if lines.is_empty() {
                            ui.label("（还没烘过）");
                        }
                        for line in lines {
                            ui.label(egui::RichText::new(line).monospace().size(11.0));
                        }
                    });
            });
    }

    fn perform(&mut self, action: Action, root: &std::path::Path) {
        match action {
            Action::Set { node, field, value } => {
                let Some(store) = self.editor.as_mut() else {
                    return;
                };
                let label = store.nodes()[node].fields[field].key.clone();
                match store.set(node, field, value) {
                    Ok(()) => {
                        *self.notice = format!(
                            "改了 {}::{} 的 `{label}`（会话副本已落盘，按 Cook 烘）",
                            store.nodes()[node].graph,
                            store.nodes()[node].node,
                        );
                    }
                    Err(err) => *self.notice = format!("改不了 `{label}`：{err}"),
                }
            }
            Action::Reset { node } => {
                let Some(store) = self.editor.as_mut() else {
                    return;
                };
                let what = format!(
                    "{}::{}",
                    store.nodes()[node].graph,
                    store.nodes()[node].node
                );
                match store.reset(node) {
                    Ok(()) => *self.notice = format!("{what} 复位成 art/ 里那一份"),
                    Err(err) => *self.notice = format!("复位 {what} 失败：{err}"),
                }
            }
            Action::Reload => {
                let Some(recipe) = self.editor.as_ref().map(|store| store.recipe().to_string())
                else {
                    *self.notice = "没有可重读的图（--edit 没给）".to_string();
                    return;
                };
                match ParamStore::open(root, &recipe) {
                    Ok(store) => {
                        *self.notice =
                            format!("重读了会话副本（{}）", store.store_root().display());
                        *self.editor = Some(store);
                    }
                    Err(err) => *self.notice = format!("重读失败：{err}"),
                }
            }
            Action::SaveToArt => {
                let Some(store) = self.editor.as_mut() else {
                    return;
                };
                match store.save_to_art() {
                    Ok(written) if written.is_empty() => {
                        *self.notice = "没有改过的节点，art/ 一个字节都没动".to_string();
                    }
                    Ok(written) => {
                        *self.notice = format!(
                            "写回 art/ {} 个文件：{}",
                            written.len(),
                            written
                                .iter()
                                .map(|path| path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                                    .unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        );
                        if let Some(store) = self.editor.take() {
                            *self.editor = ParamStore::open(root, store.recipe()).ok();
                        }
                    }
                    Err(err) => *self.notice = format!("写回 art/ 失败：{err}"),
                }
            }
            Action::Cook => {
                let Some(store) = &*self.editor else {
                    *self.notice = "没有可烘的图（--edit 没给）".to_string();
                    return;
                };
                let recipe = store.recipe().to_string();
                let pcg_root = crate::art::default_pcg_root();
                match cook::start(store, &recipe, &pcg_root) {
                    Ok(cook) => {
                        *self.notice = format!("开始烘（{} 步）", store.cook_order().len() + 1);
                        *self.cook = cook;
                    }
                    Err(err) => *self.notice = format!("起不了烘图：{err}"),
                }
            }
        }
    }
}

fn step_of(min: f64, max: f64) -> f64 {
    let span = (max - min).abs();
    if span <= 0.0 {
        return 0.0;
    }
    let step = span / 1000.0;
    if step <= 0.0 { 0.0 } else { step }
}

fn install_cjk_font(ctx: &egui::Context) {
    let Some((path, bytes)) = load_a_cjk_font() else {
        eprintln!(
            "⚠ 找不到一份带汉字的系统字体 ⇒ 面板里的汉字会渲染成空方框。\
             看一眼这几条路径在不在（任一即可）：{}",
            CJK_FONT_CANDIDATES.join(" ｜ ")
        );
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "px-cjk".to_string(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("px-cjk".to_string());
    }
    ctx.set_fonts(fonts);
    println!(
        "调参面板字体：{}（汉字用它，拉丁字母仍走 egui 自带那两份）",
        path
    );
}

const CJK_FONT_CANDIDATES: &[&str] = &[
    "C:/Windows/Fonts/msyh.ttc",
    "C:/Windows/Fonts/msyh.ttf",
    "C:/Windows/Fonts/simhei.ttf",
    "C:/Windows/Fonts/simsun.ttc",
    "C:/Windows/Fonts/YuGothM.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
];

fn load_a_cjk_font() -> Option<(String, Vec<u8>)> {
    for candidate in CJK_FONT_CANDIDATES {
        let path = std::path::Path::new(candidate);
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if !looks_like_a_font(&bytes) {
            eprintln!(
                "⚠ {candidate} 在盘上但不是字体（{} 字节）⇒ 跳过它",
                bytes.len()
            );
            continue;
        }
        return Some((candidate.to_string(), bytes));
    }
    None
}

fn looks_like_a_font(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0x00, 0x01, 0x00, 0x00]) | Some(b"true") | Some(b"OTTO") | Some(b"ttcf")
    )
}
