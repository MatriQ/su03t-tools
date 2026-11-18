//! Native firmware studio. Import/conversion jobs never access audio hardware.
use super::{audio_device, i18n::choose as tr};
use eframe::egui::{self, RichText};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
use su03t_update::{
    audio_asset::{self, Clip},
    firmware_audio,
    firmware_studio::Session,
};

enum Completed {
    Imported(Box<Session>),
    Source(Clip),
    Preview(Clip),
    Encoded { id: u32, mp3: Vec<u8> },
}
struct Work {
    cancel: Arc<AtomicBool>,
    result: Receiver<anyhow::Result<Completed>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Work {
    fn spawn(f: impl FnOnce(&AtomicBool) -> anyhow::Result<Completed> + Send + 'static) -> Self {
        let cancel = audio_asset::cancellation();
        let c = cancel.clone();
        let (tx, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(f(&c));
        });
        Self {
            cancel,
            result,
            worker: Some(worker),
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
enum StudioTab {
    #[default]
    Audio,
    Commands,
    Logging,
    Inspect,
}
pub struct Studio {
    session: Option<Session>,
    tab: StudioTab,
    pending_import: Option<PathBuf>,
    selected: Option<u32>,
    source: Option<Clip>,
    waveform: super::audio_editor::Waveform,
    commands: super::command_ui::Commands,
    start: f64,
    end: f64,
    gain: f32,
    ffmpeg: PathBuf,
    devices: Vec<(String, String)>,
    input: String,
    audio: Option<audio_device::Task>,
    recording: bool,
    work: Option<Work>,
    message: String,
    error: Option<String>,
    discard_capture: bool,
    #[cfg(debug_assertions)]
    demo: bool,
}
impl Default for Studio {
    fn default() -> Self {
        Self {
            session: None,
            tab: StudioTab::Audio,
            pending_import: None,
            selected: None,
            source: None,
            waveform: super::audio_editor::Waveform::default(),
            commands: Default::default(),
            start: 0.0,
            end: 0.0,
            gain: 1.0,
            ffmpeg: audio_asset::default_ffmpeg(),
            devices: vec![],
            input: String::new(),
            audio: None,
            recording: false,
            work: None,
            message: String::new(),
            error: None,
            discard_capture: false,
            #[cfg(debug_assertions)]
            demo: false,
        }
    }
}
impl Studio {
    pub fn busy(&self) -> bool {
        self.audio.is_some() || self.work.is_some()
    }
    fn result<T>(&mut self, result: anyhow::Result<T>) -> Option<T> {
        match result {
            Ok(value) => {
                self.error = None;
                Some(value)
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                None
            }
        }
    }
    pub fn import(&mut self, path: PathBuf) {
        if self.busy() {
            return;
        }
        if self.source.is_some()
            || self.session.as_ref().is_some_and(|s| {
                !s.modified_ids().is_empty() || s.commands_changed() || s.logging_changed()
            })
        {
            self.pending_import = Some(path);
            return;
        }
        self.begin_import(path);
    }
    fn begin_import(&mut self, path: PathBuf) {
        self.error = None;
        self.message = tr("正在检查固件…", "Inspecting firmware…").into();
        self.work = Some(Work::spawn(move |_| {
            Session::load(&path).map(|s| Completed::Imported(Box::new(s)))
        }));
    }
    fn set_source(&mut self, clip: Clip) {
        self.start = 0.0;
        self.end = clip.duration();
        self.gain = 1.0;
        self.waveform.set_source(&clip);
        self.source = Some(clip);
    }
    fn edited_wav(&self) -> anyhow::Result<Vec<u8>> {
        self.source
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No source recording"))?
            .wav(self.start, self.end, self.gain)
    }
    fn poll(&mut self) {
        let done = self.work.as_ref().and_then(|w| match w.result.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("Audio/file worker stopped")))
            }
            Err(_) => None,
        });
        if let Some(done) = done {
            let cancelled = self.work.take().unwrap().cancel.load(Ordering::Relaxed);
            self.message.clear();
            if cancelled {
                self.message = tr("已取消。", "Cancelled.").into();
            }
            if !cancelled && let Some(done) = self.result(done) {
                match done {
                    Completed::Imported(session) => {
                        self.selected = session
                            .report
                            .audio
                            .as_ref()
                            .and_then(|a| a.replies.first().map(|r| r.id));
                        self.tab = StudioTab::Audio;
                        self.commands = Default::default();
                        self.session = Some(*session);
                        self.source = None;
                    }
                    Completed::Source(clip) => self.set_source(clip),
                    Completed::Preview(clip) => {
                        self.recording = false;
                        self.audio = Some(audio_device::Task::play(clip));
                    }
                    Completed::Encoded { id, mp3 } => {
                        let result = self.session.as_mut().unwrap().replace(id, &mp3);
                        if self.result(result).is_some() {
                            self.message = tr("播报已替换到内存，可试听或导出实验固件。", "Reply replaced in memory. Preview or export the experimental update.").into();
                        }
                    }
                }
            }
        }
        let done = self.audio.as_ref().and_then(|a| match a.result.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("Audio device worker stopped")))
            }
            Err(_) => None,
        });
        if let Some(done) = done {
            self.audio = None;
            self.recording = false;
            if !self.discard_capture
                && let Some(audio_device::Outcome::Recorded(clip)) = self.result(done)
            {
                self.set_source(clip);
                self.message = tr(
                    "录音已保留。裁剪和音量调整不会改动源录音。",
                    "Recording retained. Trim and gain leave the source intact.",
                )
                .into();
            }
            self.discard_capture = false;
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) -> egui::scroll_area::ScrollAreaOutput<()> {
        self.poll();
        if let Some(path) = self.pending_import.clone() {
            egui::Modal::new(egui::Id::new("studio-discard")).show(ui.ctx(), |ui| {
                ui.heading(tr("打开另一份固件？", "Open another firmware?"));
                ui.label(tr("当前录音和内存修改会被丢弃。请先保存 WAV 或导出固件。", "The current recording and in-memory edits will be discarded. Save WAV or export firmware first."));
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                ui.horizontal(|ui| {
                    if ui.button(tr("打开并丢弃当前编辑", "Open and discard edits")).clicked() { self.pending_import = None; self.begin_import(path); }
                    if ui.button(tr("返回编辑", "Keep editing")).clicked() { self.pending_import = None; }
                });
            });
        }
        if self.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        #[cfg(debug_assertions)]
        let blocked = self.demo;
        #[cfg(not(debug_assertions))]
        let blocked = false;
        let busy = self.busy();
        ui.horizontal_wrapped(|ui| {
            ui.heading(tr("固件工作台", "Firmware studio"));
            if ui
                .add_enabled(
                    !busy && !blocked,
                    egui::Button::new(tr("打开 BIN…", "Open BIN…")),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Firmware", &["bin"])
                    .pick_file()
            {
                self.import(path);
            }
            if self.session.is_some() {
                ui.add_enabled_ui(!busy && !blocked, |ui| self.export_control(ui));
            } else {
                ui.label(tr("或拖入全量 / 升级 BIN", "or drop a full / update BIN"));
            }
        });
        if let Some(session) = &self.session {
            egui::Frame::group(ui.style())
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(
                                session
                                    .source
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                            )
                            .strong(),
                        );
                        ui.label(format!(
                            "{:.2} MiB · {} {} · {} {}",
                            session.report.bytes as f64 / 1048576.0,
                            session.report.commands.len(),
                            tr("原始指令", "original commands"),
                            session.modified_ids().len(),
                            tr("条播报修改", "edited replies")
                        ));
                        if session.commands_changed() {
                            ui.colored_label(
                                super::AMBER,
                                tr("指令待生成", "Commands need generation"),
                            );
                        }
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("SHA-256 {}", session.report.sha256))
                                .monospace()
                                .small()
                                .color(super::MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&session.report.sha256);
                });
        }
        ui.small(tr(
            "实验性 · 未上板验证 · 新识别词需兼容语法生成器",
            "Experimental · Device unverified · New words require a compatible grammar generator",
        ));
        if self.session.is_some() {
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (tab, title) in [
                        (StudioTab::Audio, tr("播报编辑", "Reply audio")),
                        (StudioTab::Commands, tr("识别指令", "Recognition commands")),
                        (StudioTab::Logging, tr("固件日志", "Firmware logging")),
                        (StudioTab::Inspect, tr("结构检查", "Inspect")),
                    ] {
                        ui.selectable_value(&mut self.tab, tab, title);
                    }
                });
            });
        }
        self.status(ui);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("studio-scroll", self.tab))
            .show(ui, |ui| {
                ui.scope(|ui| {
                    if blocked {
                        ui.visuals_mut().disabled_alpha = 1.0;
                    }
                    ui.add_enabled_ui(!busy && !blocked, |ui| self.content(ui));
                });
            })
    }
    // Reserve both rows even when idle. Child content is clipped to the
    // allocated rectangle, so long messages/meters cannot resize the viewport.
    fn status(&mut self, ui: &mut egui::Ui) {
        let row_height = ui
            .spacing()
            .interact_size
            .y
            .max(ui.text_style_height(&egui::TextStyle::Body));
        let height = row_height * 2.0 + ui.spacing().item_spacing.y;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        let busy = self.busy();
        let (text, color) = if let Some(error) = &self.error {
            (
                format!("{}: {error}", tr("操作失败", "Operation failed")),
                super::RED,
            )
        } else if !self.message.is_empty() {
            (self.message.clone(), super::INK)
        } else {
            (
                tr(
                    if busy { "处理中…" } else { "就绪" },
                    if busy { "Working…" } else { "Ready" },
                )
                .into(),
                super::MUTED,
            )
        };
        let single_line = text.replace(['\n', '\r'], " · ");
        ui.add(egui::Label::new(RichText::new(single_line).color(color)).truncate())
            .on_hover_text(&text)
            .context_menu(|ui| {
                if ui.button(tr("复制状态", "Copy status")).clicked() {
                    ui.ctx().copy_text(text.clone());
                    ui.close();
                }
            });
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                if let Some(audio) = &self.audio {
                    if ui
                        .button(if self.recording {
                            tr("停止并保留录音", "Stop and keep")
                        } else {
                            tr("停止播放", "Stop playback")
                        })
                        .clicked()
                    {
                        audio.stop();
                    }
                    if self.recording && ui.button(tr("放弃录音", "Discard take")).clicked() {
                        self.discard_capture = true;
                        audio.stop();
                    }
                    if self.recording {
                        ui.label(format!(
                            "{} {:.1} / 60 s",
                            tr("录音", "Recording"),
                            audio.seconds.load(Ordering::Relaxed) as f32 / 1000.0
                        ));
                        let peak = audio.peak.load(Ordering::Relaxed) as f32 / 1000.0;
                        ui.add(
                            egui::ProgressBar::new(peak)
                                .desired_width(90.0)
                                .text(tr("电平", "Level")),
                        );
                        if peak >= 0.98 {
                            ui.colored_label(super::RED, tr("过响 / 削波", "Loud / clipping"));
                        }
                    } else {
                        ui.label(tr("正在试听", "Playing preview"));
                    }
                }
                if let Some(work) = &self.work
                    && ui.button(tr("取消", "Cancel")).clicked()
                {
                    work.cancel.store(true, Ordering::Relaxed);
                    self.message = tr("正在取消…", "Cancelling…").into();
                }
            });
        }
    }
    fn content(&mut self, ui: &mut egui::Ui) {
        if self.session.is_none() {
            egui::Frame::group(ui.style()).inner_margin(24.0).show(ui, |ui| {
                ui.heading(tr("从已有固件开始", "Start from existing firmware"));
                ui.label(tr("打开或拖入全量 / 升级 BIN，浏览原始资源，修改播报与已支持的日志设置。", "Open or drop a full / update BIN to browse resources, edit replies and supported logging settings."));
                ui.add_space(12.0);
                ui.strong(tr("01  导入 BIN     →     02  编辑与试听     →     03  导出升级固件", "01  Import BIN     →     02  Edit and preview     →     03  Export update"));
            });
            return;
        }
        match self.tab {
            StudioTab::Audio => self.audio_content(ui),
            StudioTab::Commands => self.command_content(ui),
            StudioTab::Logging => self.logging_content(ui),
            StudioTab::Inspect => self.inspect_content(ui),
        }
    }
    fn command_content(&mut self, ui: &mut egui::Ui) {
        if let Some(session) = &mut self.session {
            ui.heading(tr("识别语音指令", "Recognition commands"));
            ui.colored_label(super::AMBER, tr("修改方案尚未写入 BIN；新词和删除识别词需匹配的语法生成器。当前仅可保存方案，不能生成含命令修改的固件。", "Draft changes are not written to the BIN. Recognition edits require a matching grammar generator. Save a plan; firmware generation with command edits is currently unavailable."));
            self.commands.set_reply_ids(
                session
                    .report
                    .audio
                    .as_ref()
                    .map(|a| a.replies.iter().map(|r| r.id).collect())
                    .unwrap_or_default(),
            );
            self.commands.ui(ui, &mut session.commands);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(tr("保存命令方案…", "Save command plan…"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name("commands.plan.json")
                        .save_file()
                {
                    let result = serde_json::to_vec_pretty(&session.command_plan())
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| firmware_audio::write_new(&path, &bytes));
                    match result {
                        Ok(()) => {
                            self.message = tr(
                                "命令方案已保存，未生成固件。",
                                "Command plan saved; no firmware generated.",
                            )
                            .into()
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                if ui
                    .button(tr("载入命令方案…", "Load command plan…"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                {
                    let result = (|| -> anyhow::Result<()> {
                        use std::io::Read;
                        let mut bytes = Vec::new();
                        std::fs::File::open(path)?
                            .take(8 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)?;
                        anyhow::ensure!(
                            bytes.len() <= 8 * 1024 * 1024,
                            "Command plan exceeds 8 MiB"
                        );
                        session.import_command_plan(serde_json::from_slice(&bytes)?)
                    })();
                    match result {
                        Ok(()) => {
                            self.commands = Default::default();
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                if ui
                    .button(tr("还原 BIN 命令", "Restore BIN commands"))
                    .clicked()
                {
                    session.restore_commands();
                    self.commands = Default::default();
                }
            });
            if session.commands_changed() {
                ui.colored_label(super::RED,tr("命令方案有修改：升级 BIN 导出已阻止。保存方案后，可还原命令以单独导出播报修改。", "Command edits pending: BIN export is blocked. Save the plan, then restore commands to export audio edits separately."));
            }
        }
        if let Some(session) = &self.session {
            egui::CollapsingHeader::new(tr(
                "命令与播报关联（只读）",
                "Commands and reply associations (read only)",
            ))
            .default_open(false)
            .show(ui, |ui| {
                if session.report.commands.is_empty() {
                    ui.label(tr("未恢复命令映射", "No command mappings recovered"));
                }
                egui::ScrollArea::vertical()
                    .id_salt("studio-commands")
                    .max_height(145.0)
                    .show(ui, |ui| {
                        egui::Grid::new("command-map")
                            .striped(true)
                            .min_col_width(65.0)
                            .show(ui, |ui| {
                                ui.strong(tr("识别词", "Phrase"));
                                ui.strong("CMD");
                                ui.strong(tr("播报 ID", "Reply IDs"));
                                ui.strong(tr("来源偏移", "Source offset"));
                                ui.end_row();
                                for command in &session.report.commands {
                                    ui.label(&command.text);
                                    ui.label(&command.command);
                                    ui.horizontal_wrapped(|ui| {
                                        if command.reply_ids.is_empty() {
                                            ui.label(&command.pcm);
                                        }
                                        for id in &command.reply_ids {
                                            let available =
                                                session.report.audio.as_ref().is_some_and(|a| {
                                                    a.replies.iter().any(|r| r.id == *id)
                                                });
                                            if ui
                                                .add_enabled(
                                                    available,
                                                    egui::Button::selectable(
                                                        self.selected == Some(*id),
                                                        id.to_string(),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                self.selected = Some(*id);
                                                self.tab = StudioTab::Audio;
                                            }
                                        }
                                    });
                                    ui.monospace(format!("0x{:x}", command.span.offset))
                                        .on_hover_text(if command.pointer_offset.is_some() {
                                            tr(
                                                "已由连续指针表交叉验证",
                                                "Corroborated by consecutive pointer array",
                                            )
                                        } else {
                                            tr(
                                                "候选常量，缺少指针证据",
                                                "Candidate constant without pointer evidence",
                                            )
                                        });
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
    }
    fn logging_content(&mut self, ui: &mut egui::Ui) {
        if let Some(session) = &mut self.session {
            egui::CollapsingHeader::new(tr("固件日志（写入导出固件）", "Firmware logging (applied to export)"))
                .id_salt("firmware-logging").default_open(true).show(ui, |ui| {
                    use su03t_update::firmware_logging::Kind;
                    let settings=session.report.logging.settings.clone();
                    let mut change=None;
                    egui::ScrollArea::horizontal().id_salt("firmware-log-scroll").auto_shrink([false,true]).show(ui,|ui| {
                    ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Extend);
                    egui::Grid::new("firmware-log-settings").min_col_width(90.0).striped(true).show(ui, |ui| {
                        for setting in &settings {
                            ui.label(match setting.kind { Kind::Runtime=>tr("应用运行日志", "Application runtime log"),Kind::Engine=>tr("语音引擎独立等级", "Independent engine log level"),Kind::Bootloader=>tr("启动程序日志", "Bootloader log") });
                            ui.label(match setting.enabled { Some(true)=>tr("开", "On"),Some(false)=>tr("关", "Off"),None=>tr("未知 / 不支持", "Unknown / unsupported") });
                            if setting.editable {
                                let mut enabled=setting.enabled.unwrap_or(false);
                                if ui.checkbox(&mut enabled,tr("启用", "Enable")).changed() {change=Some((setting.kind,enabled));}
                            } else { ui.label("—"); }
                            if let Some(raw)=setting.raw_value {
                                ui.monospace(format!("{} = {raw}",setting.config_path)).on_hover_text(setting.source_offset.map(|o|format!("0x{o:x}")).unwrap_or_default());
                            } else {ui.label("—");}
                            ui.end_row();
                        }
                    });
                    });
                    if let Some((kind,enabled))=change {
                        match session.set_logging(kind,enabled) {
                            Ok(())=>{self.error=None;self.message=tr("运行日志设置已修改到内存，将随播报一起导出。", "Runtime log setting updated in memory; export includes audio and logging edits.").into();},
                            Err(e)=>self.error=Some(format!("{e:#}")),
                        }
                    }
                    if ui.add_enabled(session.logging_changed(),egui::Button::new(tr("撤销日志修改", "Undo logging edits"))).clicked()
                        && let Err(e)=session.undo_logging() {self.error=Some(e.to_string());}
                    ui.small(tr("仅控制应用日志初始化，不保证串口静音；直接 printf、ARPT 和启动输出不在此开关范围。引擎等级设置函数未应用配置值，保持只读。", "Controls runtime logger initialization, not all UART output. Direct printf, ARPT and boot output are outside this switch. The independent engine level setter does not apply its value and stays read only."));
                });
        }
    }
    fn inspect_content(&mut self, ui: &mut egui::Ui) {
        if let Some(session) = &self.session {
            egui::CollapsingHeader::new(tr(
                "配置与结构（只读）",
                "Configuration and structure (read only)",
            ))
            .show(ui, |ui| {
                ui.label(tr(
                    "GPIO / UART 动作编码尚未恢复；不会猜测配置。",
                    "GPIO / UART action encoding is unavailable; no inferred defaults.",
                ));
                for config in &session.report.configurations {
                    egui::CollapsingHeader::new(format!(
                        "{} @ 0x{:x}",
                        config.kind, config.span.offset
                    ))
                    .show(ui, |ui| json_tree(ui, "config", &config.value));
                }
                ui.label(format!(
                    "{}: {}",
                    tr("有边界的语法资源", "Bounded grammar resources"),
                    session.report.grammar_resources.len()
                ));
                for warning in &session.report.warnings {
                    ui.small(warning);
                }
            });
        }
    }
    fn audio_content(&mut self, ui: &mut egui::Ui) {
        let replies = self.session.as_ref().and_then(|s| s.report.audio.clone());
        if let Some(layout) = replies {
            ui.horizontal_wrapped(|ui| {
                ui.strong(tr("播报资源", "Reply resources"));
                ui.label(&layout.profile);
                ui.label(format!(
                    "{} {}",
                    layout.replies.len(),
                    tr("条资源", "resources")
                ));
            });
            let commands = self
                .session
                .as_ref()
                .map(|s| s.report.commands.clone())
                .unwrap_or_default();
            let modified = self
                .session
                .as_ref()
                .map(|s| s.modified_ids())
                .unwrap_or_default();
            let mut action = None;
            egui::ScrollArea::both()
                .id_salt("studio-reply-table-scroll")
                .max_height(170.0)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    egui::Grid::new("studio-reply-table")
                        .striped(true)
                        .min_col_width(62.0)
                        .show(ui, |ui| {
                            for title in [
                                "ID",
                                tr("关联命令", "Linked commands"),
                                tr("时长", "Duration"),
                                tr("已用 / 容量", "Used / capacity"),
                                tr("状态", "Status"),
                                tr("操作", "Actions"),
                            ] {
                                ui.strong(title);
                            }
                            ui.end_row();
                            for reply in &layout.replies {
                                if ui
                                    .selectable_label(
                                        self.selected == Some(reply.id),
                                        reply.id.to_string(),
                                    )
                                    .clicked()
                                {
                                    self.selected = Some(reply.id);
                                }
                                let linked = commands
                                    .iter()
                                    .filter(|c| c.reply_ids.contains(&reply.id))
                                    .map(|c| c.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                ui.add_sized(
                                    [230.0, 22.0],
                                    egui::Label::new(if linked.is_empty() {
                                        tr("未恢复关联", "No recovered link")
                                    } else {
                                        &linked
                                    })
                                    .truncate(),
                                )
                                .on_hover_text(&linked);
                                ui.label(format!("{:.2} s", reply.duration_ms as f64 / 1000.0));
                                ui.monospace(format!("{} / {} B", reply.length, reply.capacity))
                                    .on_hover_text(format!(
                                        "MP3: {} B · @ 0x{:x}",
                                        reply.mp3_bytes, reply.offset
                                    ));
                                ui.label(if modified.contains(&reply.id) {
                                    tr("已修改", "Modified")
                                } else {
                                    tr("原始", "Original")
                                });
                                ui.horizontal(|ui| {
                                    if ui.small_button(tr("试听", "Play")).clicked() {
                                        action = Some((reply.id, false));
                                    }
                                    if ui.small_button(tr("编辑", "Edit")).clicked() {
                                        action = Some((reply.id, true));
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
            if let Some((id, edit)) = action {
                self.selected = Some(id);
                if edit {
                    self.load_reply_for_editing(id);
                } else {
                    self.preview_reply(id, false);
                }
            }
            if let Some(reply) = layout.replies.iter().find(|r| Some(r.id) == self.selected) {
                ui.label(format!(
                    "{} · {:.2} s · {} / {} B · @ 0x{:x}",
                    layout.codec,
                    reply.duration_ms as f64 / 1000.0,
                    reply.length,
                    reply.capacity,
                    reply.offset
                ));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("试听原播报", "Play original")).clicked() {
                        self.preview_reply(reply.id, true);
                    }
                    if ui.button(tr("试听当前播报", "Play current")).clicked() {
                        self.preview_reply(reply.id, false);
                    }
                    if ui
                        .button(tr("提取原 MP3…", "Extract original MP3…"))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name(format!("reply_{}.mp3", reply.id))
                            .save_file()
                    {
                        let result = self
                            .session
                            .as_ref()
                            .unwrap()
                            .reply(reply.id, true)
                            .and_then(|b| firmware_audio::write_new(&path, &b));
                        if self.result(result).is_some() {
                            self.message = tr("原播报已提取。", "Original reply extracted.").into();
                        }
                    }
                    if ui.button(tr("撤销此条修改", "Undo this reply")).clicked() {
                        let result = self.session.as_mut().unwrap().undo(reply.id);
                        self.result(result);
                    }
                });
            }
        } else if self.session.is_some() {
            ui.colored_label(super::AMBER, tr("未知音频指纹：仅检查，不提供播报替换或固件导出。", "Unknown audio fingerprint: inspection only; reply replacement and firmware export unavailable."));
        }
        ui.add_space(8.0);
        ui.strong(format!(
            "{}{}",
            tr("录音与编辑", "Record and edit"),
            self.selected
                .map(|id| format!("  →  ID {id}"))
                .unwrap_or_default()
        ));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("刷新麦克风", "Refresh microphones")).clicked()
                && let Some(devices) = self.result(audio_device::inputs())
            {
                if !devices.iter().any(|d| d.0 == self.input) {
                    self.input = devices.first().map(|d| d.0.clone()).unwrap_or_default();
                }
                self.devices = devices;
                if self.devices.is_empty() {
                    self.error = Some(
                        tr(
                            "未找到麦克风，请检查系统权限或导入 WAV。",
                            "No microphone found; check system permissions or import WAV.",
                        )
                        .into(),
                    );
                }
            }
            egui::ComboBox::from_id_salt("studio-input")
                .selected_text(
                    self.devices
                        .iter()
                        .find(|d| d.0 == self.input)
                        .map(|d| d.1.as_str())
                        .unwrap_or(tr("请先刷新", "Refresh to select")),
                )
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (id, name) in &self.devices {
                        ui.selectable_value(&mut self.input, id.clone(), name);
                    }
                });
            if ui
                .add_enabled(
                    !self.input.is_empty(),
                    egui::Button::new(tr("开始录音", "Start recording")),
                )
                .clicked()
            {
                self.error = None;
                self.discard_capture = false;
                self.recording = true;
                self.audio = Some(audio_device::Task::record(self.input.clone()));
            }
            if ui.button(tr("导入 WAV…", "Import WAV…")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("WAV", &["wav"])
                    .pick_file()
            {
                self.work = Some(Work::spawn(move |_| {
                    audio_asset::load_wav(&path).map(Completed::Source)
                }));
            }
        });
        ui.small(tr("仅点击录音 / 试听后访问音频设备。最长 60 秒，源录音留在内存，重新录音或导入会替换源。", "Audio devices activate only on record / play. Maximum 60 seconds. Source stays in memory until a new take or import."));
        if let Some(clip) = &self.source {
            let duration = clip.duration();
            self.waveform
                .ui(ui, &mut self.start, &mut self.end, &mut self.gain);
            ui.small(tr("拖动左右手柄裁剪，拖动选区平移；按住 Shift 可重新框选。右侧竖条调整音量。","Drag the handles to trim, move the selection, or Shift-drag a new range. Use the vertical fader for gain."));
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{:.2} s · {} Hz", duration, clip.sample_rate));
                ui.label(tr("起点", "Start"));
                ui.add(
                    egui::DragValue::new(&mut self.start)
                        .range(0.0..=duration)
                        .speed(0.01)
                        .suffix(" s"),
                );
                ui.label(tr("终点", "End"));
                ui.add(
                    egui::DragValue::new(&mut self.end)
                        .range(0.0..=duration)
                        .speed(0.01)
                        .suffix(" s"),
                );
                ui.add(egui::Slider::new(&mut self.gain, 0.0..=4.0).text(tr("音量", "Gain")));
                if ui
                    .small_button(tr("重置裁剪 / 音量", "Reset trim / gain"))
                    .clicked()
                {
                    self.start = 0.0;
                    self.end = duration;
                    self.gain = 1.0;
                }
            });
            self.waveform
                .normalize(&mut self.start, &mut self.end, &mut self.gain);
            let from = (self.start * clip.sample_rate as f64) as usize;
            let to = ((self.end * clip.sample_rate as f64) as usize).min(clip.samples.len());
            let peak = clip
                .samples
                .get(from..to)
                .unwrap_or_default()
                .iter()
                .fold(0f32, |p, s| p.max(s.abs()))
                * self.gain;
            if peak > 1.0 {
                ui.colored_label(
                    super::AMBER,
                    tr(
                        "此音量可能削波，请降低音量。",
                        "This gain may clip; reduce it.",
                    ),
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(tr("试听裁剪结果", "Preview edited audio"))
                    .clicked()
                {
                    let result = self.edited_wav().and_then(|w| Clip::from_wav(&w));
                    if let Some(clip) = self.result(result) {
                        self.recording = false;
                        self.audio = Some(audio_device::Task::play(clip));
                    }
                }
                if ui.button(tr("保存裁剪 WAV…", "Save edited WAV…")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name("recording.wav")
                        .save_file()
                {
                    let result = self
                        .edited_wav()
                        .and_then(|w| firmware_audio::write_new(&path, &w));
                    self.result(result);
                }
                if ui
                    .add_enabled(
                        self.selected.is_some()
                            && self
                                .session
                                .as_ref()
                                .is_some_and(|s| s.report.audio.is_some() && !s.commands_changed()),
                        egui::Button::new(tr(
                            "转换并替换选中播报",
                            "Convert and replace selected reply",
                        )),
                    )
                    .clicked()
                    && let Some(wav) = self.result(self.edited_wav())
                {
                    let id = self.selected.unwrap();
                    let exe = self.ffmpeg.clone();
                    self.work = Some(Work::spawn(move |cancel| {
                        audio_asset::encode_reply(&exe, &wav, cancel)
                            .map(|mp3| Completed::Encoded { id, mp3 })
                    }));
                }
            });
        }
        egui::CollapsingHeader::new(tr("音频转换器设置", "Audio converter settings")).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("FFmpeg: {}", self.ffmpeg.display()));
                if ui.button(tr("选择可执行文件…", "Choose executable…")).clicked() && let Some(path) = rfd::FileDialog::new().pick_file() { self.ffmpeg = path; }
            });
            ui.small(tr("需本机 FFmpeg（libmp3lame）。固定输出 16 kHz 单声道 16 kbps 裸 MP3；超出容量会拒绝。", "Requires local FFmpeg with libmp3lame. Output: raw 16 kHz mono 16 kbps MP3. Oversized replies are refused."));
        });
    }
    fn export_control(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.session
                    .as_ref()
                    .is_some_and(|s| s.report.audio.is_some() && !s.commands_changed()),
                egui::Button::new(tr("导出实验性升级固件…", "Export experimental update…")),
            )
            .on_hover_text(tr("创建新的升级 BIN 和 .manifest.json 校验报告；保留原件。识别指令有改动时，需先保存方案并生成匹配的语法资源。", "Creates a new update BIN and .manifest.json integrity report. Keep the original. Edited recognition commands require saving the plan and generating matching grammar resources first."))
            .on_disabled_hover_text(tr("处理中、未知固件或识别资源待生成时不能导出。可到「识别指令」保存方案，或还原指令后导出播报 / 日志。", "Export is unavailable while busy, for unknown firmware, or while recognition generation is pending. Save the command plan, or restore commands to export audio / logging."))
            .clicked()
        {
            let session = self.session.as_ref().unwrap();
            let stem = session
                .source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(format!("{stem}_edited_update.bin"))
                .save_file()
            {
                let result = session.export(&path);
                if let Some(manifest) = self.result(result) {
                    self.message = format!(
                        "{} {}\nSHA-256 {}",
                        tr(
                            "已导出 BIN 和 .manifest.json（实验性，未上板）：",
                            "Exported BIN and .manifest.json (experimental, device unverified):"
                        ),
                        path.display(),
                        manifest.update_sha256
                    );
                }
            }
        }
    }
    fn load_reply_for_editing(&mut self, id: u32) {
        if let Some(mp3) = self.result(self.session.as_ref().unwrap().reply(id, false)) {
            let exe = self.ffmpeg.clone();
            self.work = Some(Work::spawn(move |cancel| {
                audio_asset::decode_reply(&exe, &mp3, cancel).map(Completed::Source)
            }));
        }
    }
    fn preview_reply(&mut self, id: u32, original: bool) {
        if let Some(mp3) = self.result(self.session.as_ref().unwrap().reply(id, original)) {
            let exe = self.ffmpeg.clone();
            self.work = Some(Work::spawn(move |cancel| {
                audio_asset::decode_reply(&exe, &mp3, cancel).map(Completed::Preview)
            }));
        }
    }
    #[cfg(debug_assertions)]
    pub fn preview_commands(&mut self, adding: bool) {
        self.tab = StudioTab::Commands;
        if adding {
            self.commands.preview_add();
        }
    }
    #[cfg(debug_assertions)]
    pub fn preview(&mut self) {
        use su03t_update::firmware_inspect::{self, CommandEntry, Configuration, Span};
        let mut report = firmware_inspect::inspect(b"Synthetic UI fixture").unwrap();
        report.commands = vec![CommandEntry {
            span: Span {
                offset: 0x12340,
                length: 64,
            },
            text: tr("打开灯光", "Turn on the light").into(),
            command: "101".into(),
            pcm: "7".into(),
            reply_ids: vec![7],
            pointer_offset: Some(0x12400),
        }];
        report.configurations.push(Configuration {
            span: Span {
                offset: 0x13000,
                length: 32,
            },
            kind: "engine".into(),
            value: serde_json::json!({"sample_rate":16000}),
        });
        report.image_kind = firmware_inspect::ImageKind::Update;
        report.sha256 = "0".repeat(64);
        report.bytes = 1235304;
        report.audio = Some(firmware_audio::Layout {
            profile: "platform-b · SIMULATED".into(),
            full_image: false,
            blob_offset: 0,
            directory_offset: 65536,
            replies: (7..15)
                .map(|id| firmware_audio::Reply {
                    id,
                    offset: (id as usize - 7) * 8192,
                    length: 4096,
                    capacity: 8192,
                    mp3_bytes: 4032,
                    duration_ms: 2016,
                    directory_length_offset: 65536 + (id as usize - 7) * 12 + 8,
                })
                .collect(),
            codec: "MPEG2 Layer III / 16000 Hz / mono / 16000 bps".into(),
            experimental_edit: true,
        });
        report.logging.settings[0].enabled = Some(false);
        report.logging.settings[0].editable = true;
        report.logging.settings[0].raw_value = Some(0);
        report.logging.settings[1].raw_value = Some(6);
        // Construct via a dedicated preview constructor; this fixture can never authenticate for export.
        self.session = Some(Session::preview(report));
        self.selected = Some(7);
        self.set_source(Clip {
            sample_rate: 16000,
            samples: (0..12800)
                .map(|i| (i as f32 * 0.0864).sin() * 0.35 * ((i as f32 / 640.0).min(1.0)))
                .collect(),
        });
        self.start = 0.08;
        self.end = 0.72;
        self.demo = true;
    }
}
fn json_tree(ui: &mut egui::Ui, key: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                egui::CollapsingHeader::new(key)
                    .id_salt((key, value.to_string()))
                    .show(ui, |ui| json_tree(ui, key, value));
            }
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                json_tree(ui, &format!("[{index}]"), value);
            }
        }
        _ => {
            ui.monospace(format!("{key}: {value}"));
        }
    }
}
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    #[test]
    fn actual_pane_navigation_and_command_dialog_preserve_audio_and_block_export() {
        fn draw(
            studio: &mut Studio,
            ctx: &egui::Context,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| studio.ui(ui));
                },
            )
        }
        fn locate(shape: &egui::Shape, labels: &[&str]) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if labels.contains(&t.galley.text()) => {
                    Some(t.pos + t.galley.rect.center().to_vec2())
                }
                egui::Shape::Vec(v) => v.iter().find_map(|s| locate(s, labels)),
                _ => None,
            }
        }
        fn click(studio: &mut Studio, ctx: &egui::Context, labels: &[&str]) {
            draw(studio, ctx, vec![]); // Let a newly opened modal complete its sizing pass.
            let frame = draw(studio, ctx, vec![]);
            let pos = frame
                .shapes
                .iter()
                .find_map(|s| locate(&s.shape, labels))
                .unwrap_or_else(|| panic!("control rendered: {labels:?}"));
            draw(studio, ctx, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                draw(
                    studio,
                    ctx,
                    vec![egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    }],
                );
            }
        }
        let mut studio = Studio::default();
        studio.preview();
        studio.demo = false;
        let source = studio.source.as_ref().unwrap().samples.clone();
        let ctx = egui::Context::default();
        click(&mut studio, &ctx, &["Recognition commands", "识别指令"]);
        assert!(studio.tab == StudioTab::Commands);
        studio.preview_commands(true);
        click(&mut studio, &ctx, &["Cancel", "取消"]);
        assert!(!studio.session.as_ref().unwrap().commands_changed());
        studio.preview_commands(true);
        click(
            &mut studio,
            &ctx,
            &["Add to command list", "添加到指令列表"],
        );
        let session = studio.session.as_ref().unwrap();
        assert!(session.commands_changed());
        assert_eq!(session.commands["cmdWord"].as_array().unwrap().len(), 2);
        assert!(
            session
                .export_bytes()
                .unwrap_err()
                .to_string()
                .contains("Command edits")
        );
        click(&mut studio, &ctx, &["Firmware logging", "固件日志"]);
        assert!(studio.tab == StudioTab::Logging);
        click(&mut studio, &ctx, &["Reply audio", "播报编辑"]);
        assert!(studio.tab == StudioTab::Audio);
        assert_eq!(studio.selected, Some(7));
        assert_eq!(studio.source.as_ref().unwrap().samples, source);
        assert!(studio.devices.is_empty() && studio.audio.is_none() && !studio.busy());
    }
    #[test]
    fn source_worker_completion_and_cancel_preserve_full_recording() {
        let mut studio = Studio {
            work: Some(Work::spawn(|_| {
                Ok(Completed::Source(Clip {
                    sample_rate: 16000,
                    samples: vec![0.25; 16000],
                }))
            })),
            ..Default::default()
        };
        studio
            .work
            .as_mut()
            .unwrap()
            .worker
            .take()
            .unwrap()
            .join()
            .unwrap();
        studio.poll();
        assert_eq!(studio.source.as_ref().unwrap().samples.len(), 16000);
        studio.start = 0.25;
        studio.end = 0.75;
        studio.gain = 2.0;
        assert_eq!(
            Clip::from_wav(&studio.edited_wav().unwrap())
                .unwrap()
                .samples
                .len(),
            8000
        );
        assert_eq!(studio.source.as_ref().unwrap().samples.len(), 16000);
        studio.work = Some(Work::spawn(|_| {
            Ok(Completed::Source(Clip {
                sample_rate: 16000,
                samples: vec![0.0; 8000],
            }))
        }));
        studio
            .work
            .as_mut()
            .unwrap()
            .worker
            .take()
            .unwrap()
            .join()
            .unwrap();
        studio
            .work
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        studio.poll();
        assert_eq!(studio.source.as_ref().unwrap().samples.len(), 16000);
        assert!(!studio.busy());
        studio.import("another.bin".into());
        assert!(studio.pending_import.is_some());
        assert!(studio.work.is_none());
    }
    #[test]
    fn preview_status_keeps_viewport_and_scroll_position_stable_without_devices() {
        fn frame(
            studio: &mut Studio,
            ctx: &egui::Context,
            width: f32,
        ) -> egui::scroll_area::ScrollAreaOutput<()> {
            let mut output = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 540.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    output = Some(
                        egui::CentralPanel::default()
                            .show(ctx, |ui| studio.ui(ui))
                            .inner,
                    );
                },
            );
            output.unwrap()
        }
        for language in [
            super::super::i18n::Language::Zh,
            super::super::i18n::Language::En,
        ] {
            super::super::i18n::set(language, false);
            for width in [620.0, 1100.0] {
                let ctx = egui::Context::default();
                let mut studio = Studio::default();
                studio.preview();
                studio.demo = false; // Enable normal busy/idle styling; no input events or devices.
                for _ in 0..3 {
                    frame(&mut studio, &ctx, width);
                }
                let mut output = frame(&mut studio, &ctx, width);
                output.state.offset.y = 120.0;
                output.state.store(&ctx, output.id);
                let baseline = frame(&mut studio, &ctx, width);
                assert!((baseline.state.offset.y - 120.0).abs() < 0.1);
                let assert_stable = |output: egui::scroll_area::ScrollAreaOutput<()>| {
                    assert_eq!(
                        output.inner_rect, baseline.inner_rect,
                        "status moved viewport"
                    );
                    assert!(
                        (output.state.offset.y - baseline.state.offset.y).abs() < 0.1,
                        "status changed scroll offset"
                    );
                    assert!(
                        (output.content_size.y - baseline.content_size.y).abs() < 0.1,
                        "status changed content height"
                    );
                };
                // Decoding/conversion in flight, represented by a channel with no worker.
                let (sender, result) = mpsc::channel();
                studio.work = Some(Work {
                    cancel: audio_asset::cancellation(),
                    result,
                    worker: None,
                });
                studio.message = "Long processing status\n".repeat(30);
                assert_stable(frame(&mut studio, &ctx, width));
                studio.work = None;
                drop(sender);
                // Playback and meter updates use a synthetic task; no CPAL stream.
                let (audio, sender) = audio_device::Task::simulated();
                studio.audio = Some(audio);
                studio.message.clear();
                assert_stable(frame(&mut studio, &ctx, width));
                studio.recording = true;
                studio
                    .audio
                    .as_ref()
                    .unwrap()
                    .peak
                    .store(1000, Ordering::Relaxed);
                studio
                    .audio
                    .as_ref()
                    .unwrap()
                    .seconds
                    .store(59999, Ordering::Relaxed);
                assert_stable(frame(&mut studio, &ctx, width));
                sender.send(Ok(audio_device::Outcome::Played)).unwrap();
                assert_stable(frame(&mut studio, &ctx, width));
                assert!(!studio.busy());
                studio.error = Some("A long multiline output-device error\n".repeat(30));
                assert_stable(frame(&mut studio, &ctx, width));
                studio.error = None;
                studio.message = "Saved output\nSHA-256 hash\nmanifest path".into();
                assert_stable(frame(&mut studio, &ctx, width));
            }
        }
    }
    #[test]
    fn actual_studio_renders_both_languages_without_devices() {
        for language in [
            super::super::i18n::Language::En,
            super::super::i18n::Language::Zh,
        ] {
            super::super::i18n::set(language, false);
            let mut studio = Studio::default();
            studio.preview();
            let context = egui::Context::default();
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 1100.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| studio.ui(ui));
                },
            );
            assert!(!output.shapes.is_empty());
            assert!(studio.session.as_ref().unwrap().export_bytes().is_err());
            assert!(!studio.busy());
            assert!(studio.devices.is_empty());
            assert!(studio.audio.is_none());
        }
    }
}
