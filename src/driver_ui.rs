//! Shared startup diagnostics for upgrade and debug pages.
use super::{AMBER, INK, MUTED, RED, TEAL, card, i18n::choose as tr, label};
use eframe::egui::{self, RichText};
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use su03t_update::driver_check::{self, Family, Report, Status};

type CheckResult = Result<Report, String>;
#[derive(Default)]
pub(super) struct Checker {
    result: Option<CheckResult>,
    pending: Option<Receiver<CheckResult>>,
    ports: Option<Vec<String>>,
    dirty: bool,
    initial_complete: bool,
    attention: bool,
    open: bool,
    checked: Option<Instant>,
    summary: Option<(String, String)>,
}
impl Checker {
    pub fn start(&mut self, ctx: &egui::Context) {
        #[cfg(debug_assertions)]
        if std::env::var_os("SU03T_UI_PREVIEW").is_some() {
            return;
        }
        if self.pending.is_some() || !cfg!(target_os = "macos") {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.pending = Some(rx);
        self.dirty = false;
        std::thread::spawn(move || {
            let _ = tx.send(driver_check::scan().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    pub fn observe_ports(&mut self, ports: impl Iterator<Item = String>) {
        let ports: Vec<_> = ports.collect();
        if self.ports.as_ref().is_some_and(|before| *before != ports) {
            self.dirty = true;
        }
        self.ports = Some(ports);
    }
    pub fn poll(&mut self, ctx: &egui::Context, busy: bool) {
        let received = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("driver check worker stopped".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = received {
            self.pending = None;
            // A USB change during the scan invalidates its result. Rescan before
            // presenting a warning or claiming the driver is ready.
            if !self.dirty {
                self.accept(result);
            }
        }
        if !busy && self.dirty && self.pending.is_none() {
            self.start(ctx);
        }
        if !busy && self.attention && self.pending.is_none() && !self.dirty {
            self.open = true;
            self.attention = false;
        }
    }
    fn accept(&mut self, result: CheckResult) {
        let needs_attention = matches!(&result, Ok(r) if r.status() == Status::NeedsAttention);
        if !needs_attention {
            self.attention = false;
        } else if !self.initial_complete {
            self.attention = true;
        }
        self.initial_complete = true;
        if self.result.as_ref() != Some(&result) {
            self.summary = Some(match &result {
                Ok(report) => match report.status() {
                    Status::Ready => (
                        format!("macOS 驱动检查：{} 个 USB 转串口已创建串口接口。", report.adapters.len()),
                        format!("macOS driver check: {} USB serial adapters have serial interfaces.", report.adapters.len()),
                    ),
                    Status::NeedsAttention => (
                        "macOS 驱动检查：USB 转串口已识别，但部分设备没有串口接口。请查看驱动检查。".into(),
                        "macOS driver check: a USB serial adapter has no serial interface. Open driver check for guidance.".into(),
                    ),
                    Status::NotDetected => (
                        "macOS 驱动检查：未检测到常见 USB 转串口，暂时无法验证驱动。请连接设备后重新检查。".into(),
                        "macOS driver check: no recognized USB serial adapter; driver readiness is unverified. Connect an adapter and check again.".into(),
                    ),
                },
                Err(error) => (
                    format!("macOS 驱动检查失败（未判定驱动缺失）：{error}"),
                    format!("macOS driver check failed (driver absence is unconfirmed): {error}"),
                ),
            });
        }
        self.result = Some(result);
        self.checked = Some(Instant::now());
    }
    pub fn take_summary(&mut self) -> Option<(String, String)> {
        self.summary.take()
    }
    fn heading(&self) -> (&'static str, egui::Color32) {
        if self.pending.is_some() {
            return (tr("正在检查 macOS 驱动…", "Checking macOS drivers…"), MUTED);
        }
        if self.dirty {
            return (
                tr(
                    "设备已变化，待重新检查",
                    "Devices changed · Recheck pending",
                ),
                MUTED,
            );
        }
        match self.result.as_ref() {
            Some(Ok(report)) => match report.status() {
                Status::Ready => (tr("串口驱动已就绪", "Serial drivers ready"), TEAL),
                Status::NeedsAttention => (
                    tr(
                        "USB 已识别，串口未就绪",
                        "USB detected · Serial interface missing",
                    ),
                    AMBER,
                ),
                Status::NotDetected => (
                    tr("连接 USB 转串口后检查", "Connect a USB serial adapter"),
                    MUTED,
                ),
            },
            Some(Err(_)) => (tr("驱动检查失败", "Driver check failed"), AMBER),
            None => (tr("macOS 驱动检查", "macOS driver check"), MUTED),
        }
    }
    pub fn entry(&mut self, ui: &mut egui::Ui) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (title, color) = self.heading();
        if ui
            .small_button(RichText::new(title).color(color))
            .on_hover_text(tr(
                "查看 USB 转串口和驱动状态",
                "Inspect USB serial adapters and driver status",
            ))
            .clicked()
        {
            self.open = true;
            self.start(ui.ctx());
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        // While reading guidance, detect a newly connected adapter or a completed
        // installation without requiring the user to restart the app.
        if self.pending.is_none()
            && self
                .checked
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(5))
        {
            self.start(ctx);
        }
        let mut recheck = false;
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("macos-driver-check"))
            .frame(card().inner_margin(24))
            .show(ctx, |ui| {
                ui.set_width(560.0_f32.min((ctx.content_rect().width() - 100.0).max(280.0)));
                ui.label(RichText::new(tr("macOS 驱动检查", "macOS driver check")).size(22.0).strong().color(INK));
                ui.add_space(8.0);
                let (heading, color) = self.heading();
                ui.horizontal_wrapped(|ui| {
                    if self.pending.is_some() { ui.spinner(); }
                    ui.label(RichText::new(heading).strong().color(color));
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("driver-check-details")
                    .max_height((ctx.content_rect().height() - 260.0).max(180.0))
                    .show(ui, |ui| {
                        match &self.result {
                            Some(Ok(report)) if report.status() == Status::NotDetected => {
                                ui.label(tr(
                                    "先将 USB 转串口连接到 Mac，模块可以保持断电。未连接设备时，不能据此判断驱动缺失。",
                                    "Connect the USB serial adapter to your Mac; the module can stay powered off. An absent adapter does not establish that a driver is missing.",
                                ));
                                ui.add_space(8.0);
                                label(ui, tr("若已经连接，请检查数据线、USB 接口和转接器型号；自定义 USB ID 可能需要查阅设备厂商说明。", "If already connected, check the data cable, USB connection and adapter model. Custom USB IDs may require the device manufacturer's instructions."));
                            }
                            Some(Ok(report)) => {
                                for adapter in &report.adapters {
                                    card().inner_margin(12).show(ui, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(RichText::new(&adapter.name).strong());
                                            ui.colored_label(if adapter.ready() { TEAL } else { AMBER },
                                                if adapter.ready() { tr("串口接口已创建", "Serial interface created") } else { tr("未发现串口接口", "No serial interface") });
                                        });
                                        label(ui, &format!("USB {:04X}:{:04X}", adapter.vid, adapter.pid));
                                        for port in &adapter.ports { ui.monospace(port); }
                                        for driver in &adapter.drivers { label(ui, driver); }
                                        if !adapter.ready() {
                                            ui.add_space(6.0);
                                            ui.label(tr("USB 已识别，但尚未创建串口。驱动可能未安装或未启用，也请确认设备处于串口模式。", "USB is detected but no serial port was created. The driver may be missing or disabled; also check that the device is in serial mode."));
                                            if let Some(family) = adapter.family {
                                                ui.hyperlink_to(tr("查看厂商驱动与安装说明 ↗", "Official driver and installation guide ↗"), family.guide_url());
                                            }
                                        }
                                    });
                                    ui.add_space(8.0);
                                }
                                if report.status() == Status::NeedsAttention {
                                    ui.label(tr("按对应芯片的官方说明安装或启用驱动。若 macOS 要求允许扩展，请在系统设置的“通用 → 登录项与扩展”或“隐私与安全性”中按提示处理，再重新插入转串口。", "Follow the chip vendor's guide to install or enable the driver. If macOS requests extension approval, follow the prompt in System Settings under General → Login Items & Extensions or Privacy & Security, then reconnect the adapter."));
                                } else {
                                    ui.label(tr("串口接口已创建，可以继续选择串口。系统自带驱动与厂商驱动都支持识别。", "The serial interface is available; continue by selecting a port. Both system and vendor drivers are recognized."));
                                }
                            }
                            Some(Err(error)) => {
                                ui.label(tr("未能读取 macOS 的设备状态，暂时无法判定驱动是否就绪。可以重新检查，也可继续使用已有串口。", "macOS device status could not be read, so driver readiness is unconfirmed. Retry the check or continue using an existing serial port."));
                                ui.add_space(6.0);
                                ui.colored_label(RED, error);
                            }
                            None => { label(ui, tr("正在读取 USB 设备与串口驱动的关联…", "Reading USB devices and their serial driver bindings…")); }
                        }
                        ui.add_space(12.0);
                        label(ui, tr("常用转串口的官方资料", "Official resources for common adapters"));
                        ui.horizontal_wrapped(|ui| {
                            for family in [Family::Wch, Family::SiliconLabs, Family::Ftdi] {
                                ui.hyperlink_to(family.name(), family.guide_url());
                            }
                        });
                        ui.add_space(10.0);
                        label(ui, tr("此检查只读取系统设备信息，不打开串口、不发送数据。串口占用和访问权限在连接时检查；不代表已确认 SU-03T 型号或模块供电。", "This check reads system device information without opening a serial port or sending data. Port usage and access permissions are checked when connecting; module identity and power are not verified."));
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    recheck = ui.add_enabled(self.pending.is_none(), egui::Button::new(tr("重新检查", "Check again"))).clicked();
                    close = ui.button(tr("返回工具", "Back to tools")).clicked();
                });
            });
        if response.should_close() || close {
            self.open = false;
        }
        if recheck {
            self.start(ctx);
        }
    }
    #[cfg(all(debug_assertions, not(test)))]
    pub fn preview(&mut self, ready: bool) {
        use su03t_update::driver_check::Adapter;
        self.accept(Ok(Report {
            adapters: vec![Adapter {
                name: "SIMULATED CH340".into(),
                vid: 0x1a86,
                pid: 0x7523,
                family: Some(Family::Wch),
                ports: if ready {
                    vec!["/dev/cu.SIMULATED".into()]
                } else {
                    vec![]
                },
                drivers: if ready {
                    vec!["cn.wch.CH34xVCPDriver".into()]
                } else {
                    vec![]
                },
            }],
        }));
        self.open = true;
        self.attention = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use su03t_update::driver_check::Adapter;
    fn unbound() -> Report {
        Report {
            adapters: vec![Adapter {
                name: "test".into(),
                vid: 0x1a86,
                pid: 0x7523,
                family: Some(Family::Wch),
                ports: vec![],
                drivers: vec![],
            }],
        }
    }
    #[test]
    fn an_unbound_startup_device_prompts_once_and_rechecks_do_not_repeat_the_popup() {
        let mut checker = Checker::default();
        checker.accept(Ok(unbound()));
        assert!(checker.attention);
        checker.poll(&egui::Context::default(), false);
        assert!(checker.open);
        assert!(!checker.attention);
        checker.open = false;
        checker.accept(Ok(unbound()));
        assert!(!checker.attention);
        assert!(!checker.open);
    }
    #[test]
    fn unplugged_device_and_scan_errors_do_not_claim_a_missing_driver() {
        for result in [Ok(Report::default()), Err("test registry error".into())] {
            let mut checker = Checker::default();
            checker.accept(result);
            assert!(!checker.attention);
            assert!(!checker.open);
        }
    }
    #[test]
    fn a_deferred_startup_warning_is_cancelled_when_the_adapter_disappears() {
        let mut checker = Checker::default();
        checker.accept(Ok(unbound()));
        let ctx = egui::Context::default();
        checker.poll(&ctx, true);
        assert!(!checker.open);
        checker.accept(Ok(Report::default()));
        checker.poll(&ctx, false);
        assert!(!checker.open);
        assert!(!checker.attention);
    }
    #[test]
    fn a_scan_result_invalidated_by_unplugging_cannot_prompt_or_replace_the_status() {
        let mut checker = Checker::default();
        let (tx, rx) = mpsc::channel();
        checker.pending = Some(rx);
        checker.observe_ports(std::iter::once("/dev/cu.old".into()));
        checker.observe_ports(std::iter::empty());
        assert!(checker.dirty);
        tx.send(Ok(unbound())).unwrap();
        // A running session defers the rescan without applying the stale result.
        checker.poll(&egui::Context::default(), true);
        assert!(checker.result.is_none());
        assert!(!checker.attention);
        assert!(!checker.open);
        assert!(checker.dirty);
    }
}
