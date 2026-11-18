//! Device-free waveform gestures. Source samples are immutable; only selection
//! bounds and preview/export gain change. No device is opened by this widget.
use super::i18n::choose as tr;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Vec2, pos2};
use su03t_update::audio_asset::Clip;

#[derive(Clone, Copy)]
enum Drag {
    Start,
    End,
    Move { anchor: f64, start: f64, end: f64 },
    Select { anchor: f64 },
    Gain,
}
#[derive(Default)]
pub struct Waveform {
    envelope: Vec<(f32, f32)>,
    duration: f64,
    rate: u32,
    samples: usize,
    drag: Option<Drag>,
}
impl Waveform {
    pub fn set_source(&mut self, clip: &Clip) {
        self.duration = clip.duration();
        self.rate = clip.sample_rate;
        self.samples = clip.samples.len();
        self.drag = None;
        let step = self.samples.div_ceil(2048).max(1);
        self.envelope = clip
            .samples
            .chunks(step)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold((0f32, 0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)))
            })
            .collect();
    }
    fn snap(&self, time: f64) -> f64 {
        (time * self.rate as f64)
            .round()
            .clamp(0.0, self.samples as f64)
            / self.rate as f64
    }
    pub fn normalize(&self, start: &mut f64, end: &mut f64, gain: &mut f32) {
        if self.samples == 0 || self.rate == 0 {
            return;
        }
        let step = 1.0 / self.rate as f64;
        *start = if start.is_finite() {
            start.clamp(0.0, (self.duration - step).max(0.0))
        } else {
            0.0
        };
        *end = if end.is_finite() {
            end.clamp((*start + step).min(self.duration), self.duration)
        } else {
            self.duration
        };
        *gain = if gain.is_finite() {
            gain.clamp(0.0, 4.0)
        } else {
            1.0
        };
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        start: &mut f64,
        end: &mut f64,
        gain: &mut f32,
    ) -> (Rect, Rect) {
        let (outer, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(180.0), 154.0),
            Sense::hover(),
        );
        let plot = Rect::from_min_max(outer.min, pos2(outer.right() - 66.0, outer.bottom() - 22.0));
        let gain_rect = Rect::from_min_max(
            pos2(outer.right() - 49.0, outer.top() + 16.0),
            pos2(outer.right() - 12.0, outer.bottom() - 38.0),
        );
        let geometry = (plot, gain_rect);
        if self.samples == 0 || self.duration <= 0.0 {
            return geometry;
        }
        self.normalize(start, end, gain);
        let waveform = ui.interact(plot, ui.id().with("wave-trim"), Sense::click_and_drag());
        let level = ui.interact(
            gain_rect.expand(7.0),
            ui.id().with("wave-gain"),
            Sense::click_and_drag(),
        );
        let time_at =
            |x: f32| ((x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64 * self.duration;
        let x_at = |time: f64| plot.left() + plot.width() * (time / self.duration) as f32;
        let (pressed, down, released, position, shift) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
            )
        });
        if ui.is_enabled()
            && let Some(pointer) = position
        {
            if pressed {
                if level.hovered() {
                    self.drag = Some(Drag::Gain);
                } else if waveform.hovered() {
                    let time = self.snap(time_at(pointer.x));
                    let a = x_at(*start);
                    let b = x_at(*end);
                    self.drag = Some(if shift {
                        Drag::Select { anchor: time }
                    } else if (pointer.x - a).abs() <= 10.0
                        && (pointer.x - a).abs() <= (pointer.x - b).abs()
                    {
                        Drag::Start
                    } else if (pointer.x - b).abs() <= 10.0 {
                        Drag::End
                    } else if pointer.x > a && pointer.x < b {
                        Drag::Move {
                            anchor: time,
                            start: *start,
                            end: *end,
                        }
                    } else {
                        Drag::Select { anchor: time }
                    });
                }
            }
            if (down || released)
                && let Some(drag) = self.drag
            {
                let time = self.snap(time_at(pointer.x));
                let step = 1.0 / self.rate as f64;
                match drag {
                    Drag::Start => *start = time.min(*end - step).max(0.0),
                    Drag::End => *end = time.max(*start + step).min(self.duration),
                    Drag::Move {
                        anchor,
                        start: a,
                        end: b,
                    } => {
                        let span = b - a;
                        *start = self.snap((a + time - anchor).clamp(0.0, self.duration - span));
                        *end = (*start + span).min(self.duration);
                    }
                    Drag::Select { anchor } => {
                        *start = anchor.min(time).min(self.duration - step);
                        *end = anchor.max(time).max(*start + step).min(self.duration);
                    }
                    Drag::Gain => {
                        *gain = ((gain_rect.bottom() - pointer.y) / gain_rect.height() * 4.0)
                            .clamp(0.0, 4.0)
                    }
                }
            }
        }
        if released || !down || !ui.is_enabled() {
            self.drag = None;
        }
        self.normalize(start, end, gain);
        let painter = ui.painter_at(outer);
        let teal = super::TEAL;
        painter.rect_filled(plot, 5.0, Color32::from_rgb(231, 237, 232));
        let selection = Rect::from_min_max(
            pos2(x_at(*start), plot.top()),
            pos2(x_at(*end), plot.bottom()),
        );
        painter.rect_filled(selection, 0.0, Color32::from_rgb(205, 229, 218));
        painter.line_segment(
            [
                pos2(plot.left(), plot.center().y),
                pos2(plot.right(), plot.center().y),
            ],
            Stroke::new(1.0, Color32::from_gray(189)),
        );
        for (i, &(lo, hi)) in self.envelope.iter().enumerate() {
            let x = plot.left() + plot.width() * (i as f32 + 0.5) / self.envelope.len() as f32;
            let time = (i as f64 + 0.5) / self.envelope.len() as f64 * self.duration;
            let clipped = lo.abs().max(hi.abs()) * *gain > 1.0;
            let color = if clipped && time >= *start && time <= *end {
                super::RED
            } else if time >= *start && time <= *end {
                teal
            } else {
                super::MUTED
            };
            let scale = plot.height() * 0.42;
            painter.line_segment(
                [
                    pos2(x, plot.center().y - (hi * *gain).clamp(-1.0, 1.0) * scale),
                    pos2(x, plot.center().y - (lo * *gain).clamp(-1.0, 1.0) * scale),
                ],
                Stroke::new((plot.width() / self.envelope.len() as f32).max(1.0), color),
            );
        }
        for (time, left) in [(*start, true), (*end, false)] {
            let x = x_at(time);
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(2.0, teal),
            );
            let handle = Rect::from_center_size(pos2(x, plot.top() + 14.0), Vec2::new(12.0, 28.0));
            painter.rect_filled(handle, 3.0, teal);
            painter.line_segment(
                [
                    pos2(x - 2.0, handle.top() + 7.0),
                    pos2(x - 2.0, handle.bottom() - 7.0),
                ],
                Stroke::new(1.0, Color32::WHITE),
            );
            painter.line_segment(
                [
                    pos2(x + 2.0, handle.top() + 7.0),
                    pos2(x + 2.0, handle.bottom() - 7.0),
                ],
                Stroke::new(1.0, Color32::WHITE),
            );
            let label = if left {
                tr("起点", "Start")
            } else {
                tr("终点", "End")
            };
            painter.text(
                pos2(x, plot.bottom() - 4.0),
                if left {
                    Align2::LEFT_BOTTOM
                } else {
                    Align2::RIGHT_BOTTOM
                },
                format!("{label} {time:.3}s"),
                FontId::proportional(10.0),
                teal,
            );
        }
        painter.text(
            pos2(plot.left(), outer.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            "0.000 s",
            FontId::monospace(10.0),
            super::MUTED,
        );
        painter.text(
            pos2(plot.right(), outer.bottom() - 2.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.3} s", self.duration),
            FontId::monospace(10.0),
            super::MUTED,
        );
        painter.rect_filled(gain_rect, 5.0, Color32::from_rgb(223, 230, 224));
        let y = gain_rect.bottom() - gain_rect.height() * (*gain / 4.0);
        painter.rect_filled(
            Rect::from_min_max(pos2(gain_rect.left(), y), gain_rect.max),
            5.0,
            Color32::from_rgb(138, 188, 165),
        );
        painter.line_segment(
            [
                pos2(gain_rect.left() - 3.0, y),
                pos2(gain_rect.right() + 3.0, y),
            ],
            Stroke::new(3.0, teal),
        );
        painter.text(
            pos2(gain_rect.center().x, outer.top()),
            Align2::CENTER_TOP,
            tr("音量", "Gain"),
            FontId::proportional(10.0),
            teal,
        );
        painter.text(
            pos2(gain_rect.center().x, gain_rect.bottom() + 5.0),
            Align2::CENTER_TOP,
            format!("{:.0}%", *gain * 100.0),
            FontId::monospace(10.0),
            teal,
        );
        if waveform.hovered() {
            ui.ctx().set_cursor_icon(if self.drag.is_some() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
        if level.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        waveform.on_hover_text(tr("拖动手柄裁剪；拖动选区平移；在外侧或按住 Shift 拖动重新框选。","Drag handles to trim; drag the selection to move it; drag outside or hold Shift to draw a new selection."));
        level.on_hover_text(tr(
            "上下拖动调整音量，不会自动播放。",
            "Drag vertically to adjust gain. Playback stays explicit.",
        ));
        geometry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Editor {
        ctx: egui::Context,
        wave: Waveform,
        start: f64,
        end: f64,
        gain: f32,
    }
    impl Editor {
        fn new() -> Self {
            let mut wave = Waveform::default();
            wave.set_source(&Clip {
                sample_rate: 16000,
                samples: vec![0.5; 16000],
            });
            Self {
                ctx: egui::Context::default(),
                wave,
                start: 0.2,
                end: 0.8,
                gain: 1.0,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>, shift: bool) -> (Rect, Rect) {
            let mut geometry = (Rect::NOTHING, Rect::NOTHING);
            let _ = self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    events,
                    modifiers: egui::Modifiers {
                        shift,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        geometry = self
                            .wave
                            .ui(ui, &mut self.start, &mut self.end, &mut self.gain);
                    });
                },
            );
            geometry
        }
        fn drag(&mut self, a: egui::Pos2, b: egui::Pos2, shift: bool) {
            self.frame(vec![egui::Event::PointerMoved(a)], shift);
            self.frame(
                vec![egui::Event::PointerButton {
                    pos: a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers {
                        shift,
                        ..Default::default()
                    },
                }],
                shift,
            );
            self.frame(vec![egui::Event::PointerMoved(b)], shift);
            self.frame(
                vec![egui::Event::PointerButton {
                    pos: b,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers {
                        shift,
                        ..Default::default()
                    },
                }],
                shift,
            );
        }
    }
    #[test]
    fn pointer_handles_move_and_reverse_selection_are_sample_bounded() {
        let mut e = Editor::new();
        let (plot, _) = e.frame(vec![], false);
        let point = |t: f32| pos2(plot.left() + plot.width() * t, plot.center().y);
        e.drag(point(0.2), point(0.3), false);
        assert!((e.start - 0.3).abs() < 0.0001);
        assert!((e.end - 0.8).abs() < 0.0001);
        e.drag(point(0.8), point(0.7), false);
        assert!((e.end - 0.7).abs() < 0.0001);
        e.drag(point(0.5), point(0.9), false);
        assert!((e.start - 0.6).abs() < 0.0001);
        assert!((e.end - 1.0).abs() < 0.0001);
        e.drag(point(0.85), point(0.15), true);
        assert!((e.start - 0.15).abs() < 0.0001);
        assert!((e.end - 0.85).abs() < 0.0001);
        e.drag(point(0.15), point(1.5), false);
        assert!(e.end - e.start >= 1.0 / 16000.0 - 1e-12);
    }
    #[test]
    fn vertical_gain_drag_changes_export_without_changing_source() {
        let mut e = Editor::new();
        let source = Clip {
            sample_rate: 16000,
            samples: vec![0.5; 16000],
        };
        let (_, fader) = e.frame(vec![], false);
        e.drag(
            fader.center(),
            pos2(fader.center().x, fader.bottom() - fader.height() * 0.375),
            false,
        );
        assert!((e.gain - 1.5).abs() < 0.001);
        let output = Clip::from_wav(&source.wav(e.start, e.end, e.gain).unwrap()).unwrap();
        assert_eq!(output.samples.len(), 9600);
        assert!((output.samples[0] - 0.75).abs() < 0.0001);
        assert_eq!(source.samples, vec![0.5; 16000]);
        e.drag(
            fader.center(),
            pos2(fader.center().x, fader.top() - 500.0),
            false,
        );
        assert_eq!(e.gain, 4.0);
    }
    #[test]
    fn single_sample_and_nonfinite_values_never_produce_invalid_ranges() {
        let mut e = Editor::new();
        e.wave.set_source(&Clip {
            sample_rate: 16000,
            samples: vec![0.1],
        });
        e.start = f64::INFINITY;
        e.end = f64::NAN;
        e.gain = f32::NAN;
        e.frame(vec![], false);
        assert_eq!(e.start, 0.0);
        assert_eq!(e.end, 1.0 / 16000.0);
        assert_eq!(e.gain, 1.0);
        assert!(e.wave.drag.is_none());
        assert_eq!(e.wave.envelope.len(), 1);
    }
}
