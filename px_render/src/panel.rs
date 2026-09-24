//! **面板那一半**（S9）：把 [`crate::edit::ParamStore`] 画成控件，再接到 [`crate::cook`]。
//!
//! 它住在**交换链**上，不在判据那条路上 —— 这一条是硬约束，写在这里免得后来的人搬错：
//!
//! ```text
//! render::Session::draw → 回读出来的字节 ─┬→ Present::upload → 交换链 ─→ 屏幕上那张
//!                                        └→ shot::write_png（--shot）
//! ```
//!
//! 面板画在**交换链那一格之后**（`Viewer::draw` 里 `queue.submit` 之后、`frame.present`
//! 之前，同一个命令缓冲里的第二个 pass）。于是：
//!
//! * 面板**不可能**进 `--shot` 那张图 —— 屏幕上多了一块，图里没有；
//! * `--image-hash`（判据那把尺子）也一个字都不动。
//!
//! `--view` 那条 S7 判据（"窗口 `--shot` 与离线同文档同机位**逐字节相同**"）因此
//! **一个字都不用改**：面板是叠加，不是渲染。
//!
//! ## 输入的归属
//!
//! 鼠标/键盘**先给 egui**，它说"这一下我要了"（[`Panel::wants_pointer`]）相机就不动 ——
//! 否则在面板上拖一下滑条，相机会跟着转（而人会以为"面板坏了"）。

use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};

use crate::cook::{self, Cook};
use crate::edit::{Kind, ParamStore, Scalar};

/// 面板 + 它的设备侧资源。
pub struct Panel {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: Option<Renderer>,
    /// 状态行与日志区（`cook.rs` 给的）。
    cook: Cook,
    /// 编辑那一半（`None` = `--edit` 没给 / 开不起来，面板只说这件事）。
    editor: Option<ParamStore>,
    /// `--edit` 那一格（`None` = 没给：面板只说"怎么开它"）。
    ///
    /// ⚠ 它只用来**显示**与"这一次要不要去开 store"：真正在用的配方取的是
    ///   `store.recipe()`（面板开的哪一份就以那一份为准，没有第二个来源）。
    recipe: Option<String>,
    /// 上一次要说给用户的那句话（烘完/保存/出错都从这里出）。
    notice: String,
    /// 面板开着吗（`Tab` 切；关掉之后一个像素都不画）。
    open: bool,
}

/// 面板上的动作：**先收集、后执行**。
///
/// ⚠ 理由不是风格：控件的闭包借的是 `&ParamStore`（画那一半），而"改一个数"要 `&mut`
///   并写盘。在闭包里就地改要同时持有两个借用 —— 于是 egui 那一侧会变成一堆
///   `RefCell`/克隆。收集成一张动作表之后，两个阶段各自只持有一个借用。
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
    /// 建面板。`recipe` 是 `--edit` 给的那一格（`None` = 面板只说"怎么开它"）。
    ///
    /// ⚠ 设备侧资源（[`Renderer`]）**这里不建**：它要 `&Device`，而窗口还没开
    ///   （`Viewer::open` 才建设备）。见 [`Panel::ready`]。
    pub fn new(
        window: &winit::window::Window,
        recipe: Option<String>,
        editor: Option<ParamStore>,
    ) -> Panel {
        let ctx = egui::Context::default();
        // 深色底：面板压着的画面还要看得见（这块面板的用处就是"看着画面调参"）。
        ctx.set_visuals(egui::Visuals::dark());
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

    /// 设备侧资源建一次（`Viewer::open` 建好设备之后调）。
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
                // 面板与画面的多重采样档**故意不同**：面板走它自己的 pass（`load` 不清），
                // 与画面那张纹理没有关系。
                msaa_samples: 1,
                depth_stencil_format: None,
                ..RendererOptions::default()
            },
        ));
    }

    /// 指针**归 egui** 吗（相机的拖拽按这个让路）。
    ///
    /// ⚠ 这是个**问句**（每次现问），不是一个记下来的标志：egui 自己那条口径是
    ///   `egui_wants_pointer_input()` = "正在用指针（拖着滑条）"或"指针停在 egui 区域上
    ///   而当前没有按下任何键" —— 两档都是"这一下不是给相机的"。
    ///   记一个自己的布尔值会在"拖到面板外"那一瞬间与它漂开（而症状是相机突然跟着动）。
    pub fn wants_pointer(&self) -> bool {
        self.open && self.ctx.egui_wants_pointer_input()
    }

    /// 面板开着的（`Tab` 切换；关掉之后输入全部归相机）。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// `Tab`：收起 / 展开。
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// 窗口事件：**先给 egui**，它说"我要了"就当这一下没发生过（相机不动）。
    ///
    /// ⚠ 交回的是"egui 要了没有"，调用方按它**跳过自己那一支**（不是跳过整个事件 ——
    ///   `RedrawRequested` / `Resized` 那些根本不进这里，见 `Viewer::window_event`）。
    /// ⚠ 只把**交互**当"要了"（`consumed`）：`repaint` 那一格说的是"egui 要一帧"
    ///   （指针一动它就会响），拿它当"要了"会把相机的事件一并吞掉。
    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// 每 tick 收一次烘图进度（**非阻塞**）；交回"场景产物换成了哪一份"。
    pub fn poll(&mut self) -> Option<cook::SceneUpdate> {
        self.cook.pump()
    }

    /// 正在烘吗（窗口按这个决定要不要每 tick 要一帧：状态行上的计时在走）。
    pub fn busy(&self) -> bool {
        self.cook.busy()
    }

    /// 画面板，并把它画进 `view`（交换链那一张）。
    ///
    /// ⚠ 顺序是硬的：`run` → `update_texture` → `update_buffers` → `render`，
    ///   中间任何一步漏掉都是**静默**的空面板（egui 自己不会报）。
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
        // ---- ① 跑一遍界面（借 `self` 的全部字段）--------------------------------
        //
        // ⚠ 这一段与下面 ② 是**分开**的，理由是借用的形状：`run_ui` 的闭包要 `&mut self`，
        //   而 `Renderer::update_buffers` 要 `&mut self.renderer` —— 两个 `&mut` 撞在一起。
        //   拆成两段之后，② 里的 `self.renderer` 才能单独借出来。
        let input = self.state.take_egui_input(window);
        // ⚠ 这里**解构** `self`：`run_ui` 要借 `ctx`，而闭包要 `&mut` 其余那几个字段 ——
        //   不解构就是"同一个 `&mut self` 借两次"（E0500）。解构之后两处借的是**不同的字段**。
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

        // ---- ② 把这一批画到交换链上 -------------------------------------------
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
                // ⚠ `Load`（不是 `Clear`）：画面已经在这一张上，面板是**叠上去**的。
                //   清掉就等于"打开面板时画面变黑"，而那正是本单元最不该有的观感。
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
        // ⚠ `forget_lifetime()` **拿走** `pass`（它把生命周期抹成 `'static`，因为
        //   `Renderer::render` 要的就是那个形状）⇒ 之后不许再有 `drop(pass)`；
        //   那个 `'static` 的 pass 在 `render` 返回时自己就落了。
        renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
        // ⚠ `update_buffers` 交回的是**它自己的命令缓冲**（egui 在缓冲不够时会重新分配
        //   并录一段拷贝）。不提交就等于面板画的是**上一次**的顶点 —— 症状是面板要么
        //   空白、要么停在几帧之前，而没有任何一行会报错。
        queue.submit(user);
        for id in &full.textures_delta.free {
            renderer.free_texture(id);
        }
    }
}

/// **画界面要动的那几样**（`Panel` 里除设备侧资源之外的全部状态）。
///
/// ⚠ 它单独存在是**借用的形状**逼出来的，不是分层洁癖：`Context::run_ui` 要借 `ctx`，
///   而画界面要 `&mut` 其余状态；把后者打包成一个值，两处借的才是不同的东西
///   （否则 E0500：同一个 `&mut self` 借两次）。
struct Render<'a> {
    cook: &'a mut Cook,
    editor: &'a mut Option<ParamStore>,
    recipe: &'a Option<String>,
    notice: &'a mut String,
}

impl Render<'_> {
    /// 面板的**内容**（唯一一处描述界面长什么样的地方）。
    ///
    /// ⚠ 两点是 egui 0.35 的形状，别按旧例子写：
    ///   ① 入口是 `Context::run_ui`（交回一个根 `Ui`，不是 `Context`）⇒ 用 `show_inside`
    ///      把面板挂到那个根上；
    ///   ② 侧栏类型叫 **`Panel`**（`SidePanel` 这个名字在 0.35 已经没了 —— 左侧/右侧/
    ///      顶栏/底栏合成同一个类型，方向由 `Panel::left` / `::right` 那一族给）。
    fn ui(&mut self, ui: &mut egui::Ui, root: &std::path::Path) {
        let mut actions: Vec<Action> = Vec::new();
        egui::Panel::left("px_edit")
            .resizable(true)
            // ⚠ egui 0.35 的统一 `Panel` 用的是**尺寸**那一族名字（`default_size` /
            //   `size_range`），不是旧的 `default_width` / `width_range` —— 因为同一个
            //   类型现在也管顶栏/底栏，那一对名字在竖直方向上说不通。
            .default_size(360.0)
            .size_range(280.0..=760.0)
            .show_inside(ui, |ui| {
                self.header(ui);
                self.toolbar(ui, &mut actions, root);
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

    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, root: &std::path::Path) {
        let busy = self.cook.busy();
        let has_editor = self.editor.is_some();
        let dirty = self.editor.as_ref().is_some_and(ParamStore::dirty);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(has_editor && !busy, egui::Button::new("Cook（烘）"))
                .on_hover_text("把会话副本里的参数按顺序烘成新的场景产物，窗口随即重载")
                .clicked()
            {
                actions.push(Action::Cook);
            }
            if ui
                .add_enabled(dirty && !busy, egui::Button::new("Save → art/"))
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
            if ui
                .add_enabled(has_editor && !busy, egui::Button::new("art/ 在哪"))
                .on_hover_text(format!("{}", root.join("art").display()))
                .clicked()
            {
                *self.notice = format!(
                    "art/ 在 {}（面板不替你开资源管理器：把路径抄过去更稳）",
                    root.join("art").display()
                );
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
        // ⚠ 借用分两段：这一段只**读** store 画控件，动作收集到 `actions` 里，
        //   执行在 `Panel::ui` 的最后（那时才拿 `&mut`）。
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
                            // 类型与面板记的那一栏对不上：那是**内部**不一致，说出来。
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

    /// 动作表那一半（**唯一一处改状态的地方**）。
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
                // ⚠ 用的配方是**面板自己编辑的那一份**（`store.recipe()`），不是 `--edit`
                //   那一格：`--edit` 没给时 store 也不存在，而 store 存在时两者本来就是同一个
                //   —— 取 store 那一份，这两件事就没有"什么时候会不一致"这个格子。
                let Some(recipe) = self.editor.as_ref().map(|store| store.recipe().to_string())
                else {
                    *self.notice = "没有可重读的图（--edit 没给）".to_string();
                    return;
                };
                match ParamStore::open(root, &recipe) {
                    Ok(store) => {
                        // ⚠ 重读**不删副本**：会话里的改动还在副本上，重读只是把面板
                        //   那一份从盘上再读一遍（"我在别的编辑器里改了副本"这条也要能看见）。
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
                        // 写回之后"原值"就变了（art/ 里那一份现在等于副本）⇒ 重读一次，
                        // 否则面板会一直显示"改过"（而它已经是新的原始值了）。
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
                // ⚠ 烘的是**这份 store 对应的那份配方**（同上：不取 `--edit` 那一格）。
                let recipe = store.recipe().to_string();
                // ⚠ 烘的是**窗口正在读的那个 CAS 根**：读别处的话，面板报的键与窗口
                //   载入的那一份可以不是同一个（"看着像生效了、其实没有"）。
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

/// 滑条的步长：区间的千分之一（够细，又不至于拖不动）。
///
/// ⚠ 这一步只是**手感**：滑条本身不夹取（`Slider` 的区间是提示，不是约束 ——
///   人可以在它旁边的数字框里写任何值）。见 `edit::range_of` 那段。
fn step_of(min: f64, max: f64) -> f64 {
    let span = (max - min).abs();
    if span <= 0.0 {
        return 0.0;
    }
    let step = span / 1000.0;
    if step <= 0.0 { 0.0 } else { step }
}
