//! Firmware command draft editor.
use super::i18n::choose as tr;
use eframe::egui::{self, RichText};
use serde_json::Value;
use su03t_update::command_plan::{
    RecognitionCommand, add_recognition_command, recognition_command_issues, remove_command,
};
#[derive(Default)]
pub struct Commands {
    selected: Option<usize>,
    deleting: Option<(usize, Value)>,
    removed: Option<(usize, Value)>,
    adding: Option<RecognitionCommand>,
    search: String,
    error: Option<String>,
    reply_ids: Vec<u32>,
}
impl Commands {
    pub fn set_reply_ids(&mut self, ids: Vec<u32>) {
        self.reply_ids = ids;
    }
    pub fn open_add(&mut self) {
        self.adding = Some(RecognitionCommand::default());
        self.error = None;
    }
    #[cfg(debug_assertions)]
    pub fn preview_add(&mut self) {
        self.adding = Some(RecognitionCommand {
            phrases: tr("关闭灯光\n把灯关上", "Turn off the light\nLights off").into(),
            action: "LightOff".into(),
            reply: "7".into(),
        });
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, section: &mut Value) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(RichText::new(tr("＋ 添加识别指令", "+ Add recognition command")).strong())
                .clicked()
            {
                self.open_add();
            }
            ui.label(format!(
                "{} {}",
                section["cmdWord"].as_array().map_or(0, Vec::len),
                tr("条指令", "commands")
            ));
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(180.0)
                    .hint_text(tr(
                        "搜索识别词 / 行为 / 回复",
                        "Find phrase / action / reply",
                    )),
            );
            if ui
                .add_enabled(
                    self.removed.is_some(),
                    egui::Button::new(tr("撤销删除", "Undo deletion")),
                )
                .clicked()
                && let Some((index, row)) = self.removed.clone()
                && let Some(rows) = section.get_mut("cmdWord").and_then(Value::as_array_mut)
            {
                if rows.iter().any(|r| {
                    r["action"]
                        .as_str()
                        .zip(row["action"].as_str())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                        || r.get("num").is_some_and(|n| Some(n) == row.get("num"))
                        || r["word"].as_str().is_some_and(|words| words.split('|').map(str::trim).filter(|w| !w.is_empty()).any(|w| row["word"].as_str().unwrap_or("").split('|').map(str::trim).any(|other|w==other)))
                }) {
                    self.error = Some(
                        tr(
                            "行为、编号或识别词已被新指令使用，无法撤销删除。",
                            "A new command uses this action, ID or phrase; deletion cannot be undone.",
                        )
                        .into(),
                    );
                } else {
                    rows.insert(index.min(rows.len()), row);
                    self.removed = None;
                    self.error = None;
                    changed = true;
                }
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(super::RED, error);
        }
        if let Some(rows) = section.get_mut("cmdWord").and_then(Value::as_array_mut) {
            egui::ScrollArea::both()
                .id_salt("command-editor-table")
                .max_height(195.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_width(660.0);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    egui::Grid::new("command-editor-rows")
                        .min_col_width(110.0)
                        .striped(true)
                        .show(ui, |ui| {
                            for title in [
                                tr("行为名称", "Action"),
                                tr("识别词 / 同义说法", "Phrases / aliases"),
                                tr("回复 / 资源", "Reply / resource"),
                                tr("操作", "Actions"),
                            ] {
                                ui.strong(title);
                            }
                            ui.end_row();
                            for (index, row) in rows.iter_mut().enumerate() {
                                let query = self.search.trim().to_lowercase();
                                if !query.is_empty()
                                    && !["action", "word", "reply"].iter().any(|k| {
                                        row[*k]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_lowercase()
                                            .contains(&query)
                                    })
                                {
                                    continue;
                                }
                                for (key, width) in
                                    [("action", 115.0), ("word", 180.0), ("reply", 145.0)]
                                {
                                    if let Some(object) = row.as_object_mut() {
                                        let value = object
                                            .entry(key)
                                            .or_insert(Value::String(String::new()));
                                        if let Some(text) = value.as_str() {
                                            let mut text = text.to_owned();
                                            if ui
                                                .add(
                                                    egui::TextEdit::singleline(&mut text)
                                                        .desired_width(width),
                                                )
                                                .changed()
                                            {
                                                *value = Value::String(text);
                                                changed = true;
                                            }
                                        } else {
                                            ui.label(RichText::new(value.to_string()).monospace());
                                        }
                                    } else {
                                        ui.label("?");
                                    }
                                }
                                ui.horizontal(|ui| {
                                    if ui.small_button(tr("详情", "Details")).clicked() {
                                        self.selected = Some(index);
                                    }
                                    if ui.small_button(tr("删除", "Delete")).clicked() {
                                        self.deleting = Some((index, row.clone()));
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
            if let Some(index) = self.selected.filter(|&i| i < rows.len()) {
                egui::CollapsingHeader::new(tr("所选命令完整字段", "Selected command fields"))
                    .id_salt(index)
                    .show(ui, |ui| {
                        let fields = serde_json::to_string_pretty(&rows[index]).unwrap_or_default();
                        ui.add(
                            egui::TextEdit::multiline(&mut fields.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            }
        } else {
            ui.label(tr(
                "命令方案的 cmdWord 结构无效，请还原 BIN 指令。",
                "Invalid cmdWord structure. Restore commands from the BIN.",
            ));
        }
        if let Some((index, snapshot)) = self.deleting.clone() {
            egui::Modal::new(ui.id().with("delete-command")).show(ui.ctx(), |ui| {
                ui.heading(tr("删除这条命令？", "Delete this command?"));
                ui.label(snapshot.get("action").and_then(Value::as_str).unwrap_or("?"));
                ui.label(tr("保留控制配置和播报资源；删除识别词需重新生成语法与固件。", "Controls and audio resources are retained. Recognition changes require grammar and firmware generation."));
                ui.horizontal(|ui| {
                    if ui.button(tr("确认删除", "Confirm deletion")).clicked() {
                        if section.get("cmdWord").and_then(|v|v.get(index)) == Some(&snapshot)
                            && let Ok(row) = remove_command(section,index)
                        { self.removed=Some((index,row)); self.selected=None; changed=true; }
                        self.deleting=None;
                    }
                    if ui.button(tr("取消", "Cancel")).clicked() { self.deleting=None; }
                });
            });
        }
        if let Some(mut draft) = self.adding.take() {
            let mut close = false;
            egui::Modal::new(ui.id().with("add-recognition-command")).show(ui.ctx(),|ui| {
                ui.set_width(430.0_f32.min((ui.ctx().content_rect().width()-60.0).max(240.0)));
                ui.heading(tr("添加识别语音指令", "Add recognition command"));
                ui.label(tr("说出任一识别词时，触发同一个行为。", "Each spoken variant triggers the same action."));
                let mut valid=false;
                egui::ScrollArea::vertical().id_salt("recognition-form-scroll").max_height((ui.ctx().content_rect().height()-175.0).max(100.0)).show(ui,|ui| {
                ui.add_space(8.0);
                ui.strong(tr("识别词与同义说法", "Recognition phrases and aliases"));
                ui.add(egui::TextEdit::multiline(&mut draft.phrases).desired_rows(3).desired_width(f32::INFINITY).hint_text(tr("打开灯光\n把灯打开", "Turn on the light\nLights on")));
                ui.small(tr("每行一条，或用 | 分隔。这里只输入文字，无需录音。", "One phrase per line, or separate with |. Enter text; no recording is needed."));
                ui.label(tr("行为名称（可选，留空自动分配）", "Action name (optional, assigned if empty)"));
                ui.add(egui::TextEdit::singleline(&mut draft.action).desired_width(f32::INFINITY).hint_text("Command1"));
                ui.label(tr("关联已有播报资源（可选）", "Link an existing reply resource (optional)"));
                egui::ComboBox::from_id_salt("new-command-reply").selected_text(if draft.reply.is_empty(){tr("不指定", "Unassigned")}else{&draft.reply}).show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.reply,String::new(),tr("不指定", "Unassigned"));
                    for id in &self.reply_ids {ui.selectable_value(&mut draft.reply,id.to_string(),format!("ID {id}"));}
                });
                let issues=recognition_command_issues(section,&draft);
                valid = !issues.iter().any(|i|!i.warning);
                for issue in &issues {
                    ui.colored_label(if !issue.warning {super::RED}else{super::AMBER}, issue_label(issue.code));
                }
                ui.add_space(8.0);
                ui.small(tr("添加后保存命令方案。新词需兼容的语法资源生成器才能写入固件生效；回复录音不会改变识别词。", "Save additions in a command plan. New words need a compatible grammar generator before they can work in firmware. Reply recordings do not change recognition."));
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid,egui::Button::new(tr("添加到指令列表", "Add to command list"))).clicked() {
                        match add_recognition_command(section,&draft) {
                            Ok(index)=>{self.selected=Some(index);self.search.clear();self.error=None;changed=true;close=true;},
                            Err(e)=>self.error=Some(e.to_string()),
                        }
                    }
                    if ui.button(tr("取消", "Cancel")).clicked() {close=true;}
                });
            });
            if !close {
                self.adding = Some(draft);
            }
        }
        changed
    }
}
fn issue_label(code: &str) -> &str {
    match code {
        "empty_phrase" => tr(
            "请输入至少一条识别词。",
            "Enter at least one recognition phrase.",
        ),
        "duplicate_phrase" => tr(
            "识别词重复，请检查已有指令和同义说法。",
            "A phrase duplicates an existing command or alias.",
        ),
        "invalid_action" => tr(
            "行为名以英文字母开头，仅限字母、数字和下划线，最长 15 字符。",
            "Action names start with a letter, use letters/digits/underscore, and have at most 15 characters.",
        ),
        "duplicate_action" => tr(
            "行为名已使用（不区分大小写），请更换或留空。",
            "Action name is already used (case insensitive); change it or leave empty.",
        ),
        "control_character" => tr(
            "识别词不能包含控制字符。",
            "Recognition phrases cannot contain control characters.",
        ),
        "command_limit" => tr(
            "已达到编辑器的 1000 条指令上限。",
            "The editor limit of 1000 commands has been reached.",
        ),
        "historical_phrase_length" => tr(
            "建议中文识别词为 2–10 字；实际限制由所选 SDK 决定。",
            "Historical guidance recommends 2–10 characters; actual limits depend on the SDK.",
        ),
        "historical_phrase_characters" => tr(
            "包含英文或数字：请确认所选 SDK 的语言支持。",
            "Letters/digits detected: confirm the selected SDK's language support.",
        ),
        _ => tr(
            "命令结构无效，请检查 JSON。",
            "Invalid command structure; check the JSON.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_window_keeps_submit_and_cancel_inside_the_modal_viewport() {
        for language in [
            super::super::i18n::Language::Zh,
            super::super::i18n::Language::En,
        ] {
            super::super::i18n::set(language, false);
            let ctx = egui::Context::default();
            let mut editor = Commands::default();
            editor.preview_add();
            let mut section = serde_json::json!({"cmdWord":[]});
            let mut output = None;
            for _ in 0..4 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(540.0, 420.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            editor.ui(ui, &mut section);
                        });
                    },
                ));
            }
            for labels in [
                ["Add to command list", "添加到指令列表"],
                ["Cancel", "取消"],
            ] {
                let pos = text_position(output.as_ref().unwrap(), &labels);
                assert!(
                    (0.0..540.0).contains(&pos.x) && (0.0..420.0).contains(&pos.y),
                    "{pos:?}"
                );
            }
            assert!(section["cmdWord"].as_array().unwrap().is_empty());
        }
    }
    fn frame(
        ctx: &egui::Context,
        editor: &mut Commands,
        section: &mut Value,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    editor.ui(ui, section);
                });
            },
        )
    }
    fn text_position(output: &egui::FullOutput, labels: &[&str]) -> egui::Pos2 {
        fn search(shape: &egui::Shape, labels: &[&str]) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text)
                    if labels.iter().any(|label| text.galley.text() == *label) =>
                {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| search(shape, labels)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| search(&shape.shape, labels))
            .expect("button rendered")
    }
    fn click(ctx: &egui::Context, editor: &mut Commands, section: &mut Value, labels: &[&str]) {
        let output = frame(ctx, editor, section, vec![]);
        let pos = text_position(&output, labels);
        frame(ctx, editor, section, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                ctx,
                editor,
                section,
                vec![egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                }],
            );
        }
    }
    #[test]
    fn actual_add_delete_confirm_and_undo_buttons_edit_only_commands() {
        let ctx = egui::Context::default();
        let mut editor = Commands::default();
        let mut section = serde_json::json!({"cmdWord":[],"cmdReply":[{"preserve":42}]});
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["+ Add recognition command", "＋ 添加识别指令"],
        );
        assert!(section["cmdWord"].as_array().unwrap().is_empty());
        // Empty requests cannot create a row, even if the disabled submit is clicked.
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["Add to command list", "添加到指令列表"],
        );
        assert!(section["cmdWord"].as_array().unwrap().is_empty());
        // Type through the real multiline editor, then submit the dialog.
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["打开灯光\n把灯打开", "Turn on the light\nLights on"],
        );
        frame(
            &ctx,
            &mut editor,
            &mut section,
            vec![egui::Event::Text("打开灯光\n把灯打开".into())],
        );
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["Add to command list", "添加到指令列表"],
        );
        assert_eq!(section["cmdWord"].as_array().unwrap().len(), 1);
        assert_eq!(section["cmdWord"][0]["word"], "打开灯光|把灯打开");
        let row = section["cmdWord"][0].clone();
        click(&ctx, &mut editor, &mut section, &["Delete", "删除"]);
        assert_eq!(section["cmdWord"].as_array().unwrap().len(), 1);
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["Confirm deletion", "确认删除"],
        );
        assert!(section["cmdWord"].as_array().unwrap().is_empty());
        click(
            &ctx,
            &mut editor,
            &mut section,
            &["Undo deletion", "撤销删除"],
        );
        assert_eq!(section["cmdWord"][0], row);
        assert_eq!(section["cmdReply"][0]["preserve"], 42);
    }
}
