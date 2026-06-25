//! eframe 设置/状态窗口 + 菜单栏托盘。极简、深色、SF Pro 字体、圆角卡片。
//! eframe 0.34：logic()=每帧后台逻辑（隐藏到托盘也跑），ui()=往根 Ui 画。

use crate::app::{AppConfig, EngineState, Preset, Shared};
use eframe::egui;
use egui::{Color32, CornerRadius, FontId, Margin, RichText, Stroke};
use std::sync::Arc;
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

// 调色板（Apple 深色风）
const BG: Color32 = Color32::from_rgb(24, 24, 26);
const CARD: Color32 = Color32::from_rgb(38, 38, 41);
const CARD_HI: Color32 = Color32::from_rgb(52, 52, 56);
const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
const WEAK: Color32 = Color32::from_rgb(142, 142, 147);
const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
const WARN: Color32 = Color32::from_rgb(255, 159, 10);

pub struct SaySoApp {
    shared: Shared,
    draft: AppConfig,
    save_msg: String,
    tray: Option<TrayIcon>,
    show_id: Option<MenuId>,
    quit_id: Option<MenuId>,
    tray_tried: bool, // 托盘延迟到首帧再建：App::new 时 NSApp 还没启动完，建了不显示
    last_state: Option<EngineState>,
}

impl SaySoApp {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Shared) -> Self {
        apply_theme(&cc.egui_ctx);

        // 关键：窗口收进托盘隐藏后，eframe 默认不再调 logic()/ui()，于是托盘菜单不被处理
        // （显示/退出失灵=僵尸）、状态图标也不刷新。后台线程定时 request_repaint()，
        // 把事件循环一直叫醒，logic() 持续跑——托盘可用、图标随状态变。
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(120));
            ctx.request_repaint();
        });

        let draft = shared.config_snapshot();
        Self {
            shared,
            draft,
            save_msg: String::new(),
            tray: None,
            show_id: None,
            quit_id: None,
            tray_tried: false,
            last_state: None,
        }
    }

    /// 首帧再建托盘（此时 NSApp 已启动完，状态栏图标才会真正出现）。
    fn ensure_tray(&mut self) {
        if self.tray_tried {
            return;
        }
        self.tray_tried = true;
        match build_tray() {
            Ok((t, s, q)) => {
                self.tray = Some(t);
                self.show_id = Some(s);
                self.quit_id = Some(q);
                self.last_state = None; // 强制下一帧刷新图标颜色
            }
            Err(e) => tracing::warn!("tray icon unavailable: {e}"),
        }
    }

    fn commit(&mut self, msg: &str) {
        {
            let mut cfg = self.shared.config.lock().unwrap_or_else(|e| e.into_inner());
            *cfg = self.draft.clone();
        }
        match self.draft.save() {
            Ok(()) => self.save_msg = msg.to_string(),
            Err(e) => self.save_msg = format!("保存失败: {e}"),
        }
    }

    fn pump_tray_menu(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if Some(&ev.id) == self.show_id.as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if Some(&ev.id) == self.quit_id.as_ref() {
                std::process::exit(0);
            }
        }
    }

    fn sync_tray_icon(&mut self, state: EngineState) {
        if self.last_state == Some(state) {
            return;
        }
        self.last_state = Some(state);
        if let Some(tray) = &self.tray {
            if let Some(icon) = dot_icon(state.color()) {
                let _ = tray.set_icon(Some(icon));
            }
            let _ = tray.set_tooltip(Some(format!("SaySo — {}", state.label())));
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("输出风格").color(WEAK).size(12.0));
        ui.label(RichText::new("每条只写一行风格描述，清理纪律已内置；留空=纯清理").color(WEAK).size(11.0));
        ui.add_space(6.0);

        let mut to_delete = None;
        let n = self.draft.presets.len();
        for i in 0..n {
            card(ui.style(), CARD).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.draft.presets[i].name)
                            .desired_width(130.0)
                            .hint_text("名字"),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if n > 1 && ui.button("删除").clicked() {
                            to_delete = Some(i);
                        }
                    });
                });
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.presets[i].style)
                        .desired_width(f32::INFINITY)
                        .hint_text("例：改写成西海岸口语英语 / 翻译成日本网络用语"),
                );
            });
            ui.add_space(8.0);
        }
        if ui.button("＋ 新建风格").clicked() {
            self.draft.presets.push(Preset {
                name: "新风格".into(),
                style: String::new(),
            });
        }
        if let Some(i) = to_delete {
            self.draft.presets.remove(i);
            if self.draft.active_preset >= self.draft.presets.len() {
                self.draft.active_preset = self.draft.presets.len().saturating_sub(1);
            }
        }

        ui.add_space(12.0);
        ui.label(RichText::new("其它").color(WEAK).size(12.0));
        ui.add_space(4.0);
        egui::Grid::new("adv").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("热键");
            ui.vertical(|ui| {
                ui.text_edit_singleline(&mut self.draft.hotkey);
                ui.label(RichText::new("改键位需重启").color(WEAK).size(10.0));
            });
            ui.end_row();

            ui.label("触发");
            egui::ComboBox::from_id_salt("trigger")
                .selected_text(if self.draft.trigger_mode == "toggle" { "切换" } else { "长按" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.draft.trigger_mode, "hold".into(), "长按（按住说话）");
                    ui.selectable_value(&mut self.draft.trigger_mode, "toggle".into(), "切换（点开/点停）");
                });
            ui.end_row();

            ui.label("LLM 润色");
            ui.checkbox(&mut self.draft.llm_enabled, "开启");
            ui.end_row();

            ui.label("API Key");
            ui.add(
                egui::TextEdit::singleline(&mut self.draft.api_key)
                    .password(true)
                    .hint_text("留空用环境变量 GROQ_API_KEY"),
            );
            ui.end_row();

            ui.label("代理");
            ui.add(
                egui::TextEdit::singleline(&mut self.draft.proxy)
                    .hint_text("http://127.0.0.1:7890，空=直连"),
            );
            ui.end_row();

            ui.label("LLM 模型");
            ui.text_edit_singleline(&mut self.draft.llm_model);
            ui.end_row();
            ui.label("STT 模型");
            ui.text_edit_singleline(&mut self.draft.stt_model);
            ui.end_row();
        });

        ui.add_space(6.0);
        ui.collapsing("更冷门的", |ui| {
            egui::Grid::new("adv2").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("静音阈值");
                ui.add(egui::Slider::new(&mut self.draft.silence_threshold, 0..=3000));
                ui.end_row();
                ui.label("噪声 min_logprob");
                ui.add(egui::Slider::new(&mut self.draft.stt_min_logprob, -2.0..=0.0));
                ui.end_row();
                ui.label("LLM base_url");
                ui.text_edit_singleline(&mut self.draft.llm_base_url);
                ui.end_row();
                ui.label("STT base_url");
                ui.text_edit_singleline(&mut self.draft.stt_base_url);
                ui.end_row();
            });
            ui.label(RichText::new("STT 引导 prompt（空=关）").color(WEAK).size(11.0));
            ui.add(
                egui::TextEdit::multiline(&mut self.draft.stt_prompt)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if accent_button(ui, "保存").clicked() {
                self.commit("已保存。下次说话生效；改了热键键位需重启。");
            }
            if ui.button("恢复默认").clicked() {
                self.draft = AppConfig::default();
                self.commit("已恢复默认并保存。");
            }
        });
        if !self.save_msg.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&self.save_msg).color(WEAK).size(11.0));
        }
    }
}

impl eframe::App for SaySoApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_tray();
        ctx.request_repaint_after(Duration::from_millis(200));
        self.pump_tray_menu(ctx);
        // 关窗不退出：最小化到程序坞，后台继续跑（点程序坞里的缩略图即可还原）。
        // 真正退出走窗口里的「退出」按钮或托盘菜单。
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let state = self.shared.with_status(|s| s.state);
        self.sync_tray_icon(state);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (state, last_final, last_error, perm) = self.shared.with_status(|s| {
            (s.state, s.last_final.clone(), s.last_error.clone(), s.permission_problem)
        });

        // 统一外边距，别让内容贴边
        egui::Frame::group(ui.style())
            .fill(BG)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::ZERO)
            .inner_margin(Margin::symmetric(20, 16))
            .outer_margin(Margin::same(0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // 标题 + 右侧状态药丸
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SaySo").size(22.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        status_pill(ui, state);
                    });
                });
                ui.add_space(16.0);

                // 输出风格（核心）
                card(ui.style(), CARD).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("输出风格").color(WEAK).size(12.0));
                    ui.add_space(8.0);
                    let names: Vec<String> = self.draft.presets.iter().map(|p| p.name.clone()).collect();
                    let active = self.draft.active_preset.min(names.len().saturating_sub(1));
                    let prev = self.draft.active_preset;
                    egui::ComboBox::from_id_salt("active_preset")
                        .selected_text(
                            RichText::new(names.get(active).cloned().unwrap_or_default()).size(16.0),
                        )
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (i, nm) in names.iter().enumerate() {
                                ui.selectable_value(&mut self.draft.active_preset, i, nm);
                            }
                        });
                    if self.draft.active_preset != prev {
                        self.commit("已切换输出风格。");
                    }
                });
                ui.add_space(12.0);

                // 最近结果
                if !last_final.is_empty() {
                    card(ui.style(), CARD).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("最近").color(WEAK).size(12.0));
                        ui.add_space(4.0);
                        ui.label(RichText::new(&last_final).size(15.0).color(TEXT));
                    });
                    ui.add_space(12.0);
                }

                if perm || !last_error.is_empty() {
                    ui.colored_label(WARN, &last_error);
                    ui.add_space(8.0);
                }

                ui.collapsing(RichText::new("设置").size(13.0).color(WEAK), |ui| {
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        self.settings_ui(ui);
                    });
                });

                ui.add_space(12.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("按住右 Option 说话 · 录音时菜单栏麦克风变橙 · 关窗=最小化到程序坞")
                            .color(WEAK)
                            .size(10.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("退出").clicked() {
                            std::process::exit(0);
                        }
                    });
                });
            });
    }
}

// ── 视觉小工具 ──

/// 圆角卡片：无边框、柔和投影、留内边距，做出层次感。
fn card(style: &egui::Style, fill: Color32) -> egui::Frame {
    egui::Frame::group(style)
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .outer_margin(Margin::same(0))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(40),
        })
}

/// 右上角状态药丸：状态色淡底 + 同色圆点 + 文字。
fn status_pill(ui: &mut egui::Ui, state: EngineState) {
    let c = state.color();
    let col = Color32::from_rgb(c[0], c[1], c[2]);
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 38))
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(11))
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("● {}", state.label())).color(col).size(12.0));
        });
}

/// 蓝色强调按钮。
fn accent_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(ACCENT))
}

fn dot_icon(rgb: [u8; 3]) -> Option<Icon> {
    const N: i32 = 32;
    let r = 14.0_f32;
    let c = (N as f32 - 1.0) / 2.0;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() - r).clamp(-1.0, 1.0);
            let a = ((0.5 - d * 0.5) * 255.0) as u8;
            let i = ((y * N + x) * 4) as usize;
            rgba[i] = rgb[0];
            rgba[i + 1] = rgb[1];
            rgba[i + 2] = rgb[2];
            rgba[i + 3] = a;
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32).ok()
}

fn build_tray() -> anyhow::Result<(TrayIcon, MenuId, MenuId)> {
    let menu = Menu::new();
    let show = MenuItem::new("显示窗口", true, None);
    let quit = MenuItem::new("退出 SaySo", true, None);
    menu.append(&show)?;
    menu.append(&quit)?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("SaySo");
    if let Some(icon) = dot_icon(EngineState::Idle.color()) {
        builder = builder.with_icon(icon);
    }
    Ok((builder.build()?, show_id, quit_id))
}

/// 主题：SF Pro 字体 + 深色调色板 + 圆角 + 字号层级。
fn apply_theme(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(18, 18, 20);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(12);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(10, 132, 255, 90);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    let stroke = Stroke::new(1.0, Color32::from_rgb(58, 58, 60));
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = stroke;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, WEAK);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(12);

    v.widgets.inactive.bg_fill = CARD;
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = CornerRadius::same(10);

    v.widgets.hovered.bg_fill = CARD_HI;
    v.widgets.hovered.weak_bg_fill = CARD_HI;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.corner_radius = CornerRadius::same(10);

    v.widgets.active.bg_fill = CARD_HI;
    v.widgets.active.weak_bg_fill = CARD_HI;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.corner_radius = CornerRadius::same(10);

    v.widgets.open.bg_fill = CARD;
    v.widgets.open.corner_radius = CornerRadius::same(10);

    let mut style = (*ctx.global_style()).clone();
    style.visuals = v;
    use egui::{FontFamily, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
    ]
    .into_iter()
    .collect();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_global_style(style);
}

/// 主字体 SF Pro（拉丁，原生苹果观感）+ 中文兜底。
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut prepend: Vec<String> = Vec::new();

    if let Ok(b) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts.font_data.insert("sf".into(), Arc::new(egui::FontData::from_owned(b)));
        prepend.push("sf".into());
    }
    const CJK: &[&str] = &[
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ];
    let mut cjk_loaded = false;
    for p in CJK {
        if let Ok(b) = std::fs::read(p) {
            fonts.font_data.insert("cjk".into(), Arc::new(egui::FontData::from_owned(b)));
            prepend.push("cjk".into());
            cjk_loaded = true;
            tracing::info!("CJK font: {p}");
            break;
        }
    }
    // 把 sf、cjk 插到 Proportional 最前（保留 egui 默认做 emoji/兜底）
    let prop = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    for name in prepend.iter().rev() {
        prop.insert(0, name.clone());
    }
    if cjk_loaded {
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk".into());
    }
    ctx.set_fonts(fonts);
}
