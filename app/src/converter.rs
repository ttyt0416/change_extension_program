use crate::BACKGROUND;
use convert_engine::Category as EngineCategory;
use eframe::egui::{self, Align2, Color32, CornerRadius, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

const FILE_COLUMN_WIDTH: f32 = 220.0;
const EXTENSION_COLUMN_WIDTH: f32 = 110.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Category {
    #[default]
    Document,
    Video,
    Image,
    Audio,
}

impl Category {
    const ALL: [Self; 4] = [Self::Document, Self::Video, Self::Image, Self::Audio];

    fn label(self) -> &'static str {
        match self {
            Self::Document => "문서",
            Self::Video => "동영상",
            Self::Image => "이미지",
            Self::Audio => "소리",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Document => 0,
            Self::Video => 1,
            Self::Image => 2,
            Self::Audio => 3,
        }
    }

    fn formats(self) -> &'static [&'static str] {
        match self {
            Self::Document => convert_engine::document::OUTPUT_FORMATS,
            Self::Video => convert_engine::video::OUTPUT_FORMATS,
            Self::Image => convert_engine::image::OUTPUT_FORMATS,
            Self::Audio => convert_engine::audio::OUTPUT_FORMATS,
        }
    }

    fn formats_for(self, path: &Path) -> &'static [&'static str] {
        match self {
            Self::Document => convert_engine::document::output_formats_for(path),
            Self::Video => convert_engine::video::output_formats_for(path),
            Self::Image => convert_engine::image::output_formats_for(path),
            Self::Audio => convert_engine::audio::output_formats_for(path),
        }
    }
}

struct SelectedFile {
    path: PathBuf,
    target_extension: Option<String>,
    status: ConversionStatus,
    formats: &'static [&'static str],
}

struct FormatResult {
    category: Category,
    path: PathBuf,
    formats: &'static [&'static str],
}

#[derive(Default)]
enum ConversionStatus {
    #[default]
    Ready,
    Running,
    Complete(PathBuf),
    Failed(String),
}

struct ConversionResult {
    category: Category,
    path: PathBuf,
    result: Result<PathBuf, String>,
}

pub(crate) struct ConverterPage {
    category: Category,
    files: [Vec<SelectedFile>; 4],
    batch_extensions: [Option<String>; 4],
    output_folder: Option<PathBuf>,
    settings_open: bool,
    result_sender: Sender<ConversionResult>,
    result_receiver: Receiver<ConversionResult>,
    format_sender: Sender<FormatResult>,
    format_receiver: Receiver<FormatResult>,
}

impl Default for ConverterPage {
    fn default() -> Self {
        let (result_sender, result_receiver) = mpsc::channel();
        let (format_sender, format_receiver) = mpsc::channel();
        Self {
            category: Category::default(),
            files: std::array::from_fn(|_| Vec::new()),
            batch_extensions: std::array::from_fn(|_| None),
            output_folder: None,
            settings_open: false,
            result_sender,
            result_receiver,
            format_sender,
            format_receiver,
        }
    }
}

impl eframe::App for ConverterPage {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show_menu_bar(ui);
        self.show(ui);
        self.show_settings_window(ui.ctx());
    }
}

impl ConverterPage {
    pub(crate) fn show_menu_bar(&mut self, root_ui: &mut Ui) {
        egui::Panel::top("menu_bar")
            .frame(
                egui::Frame::NONE
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(root_ui, |ui| {
                apply_text_contrast(ui);
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("편집", |ui| {
                        apply_text_contrast(ui);
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("설정").color(Color32::BLACK),
                                )
                                .fill(Color32::from_gray(220)),
                            )
                            .clicked()
                        {
                            self.settings_open = true;
                            ui.close();
                        }
                    });
                });
            });
    }

    pub(crate) fn show(&mut self, root_ui: &mut Ui) {
        self.collect_results();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BACKGROUND).inner_margin(24.0))
            .show(root_ui, |ui| {
                apply_text_contrast(ui);
                self.content(ui);
            });
    }

    pub(crate) fn show_settings_window(&mut self, ctx: &egui::Context) {
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new(egui::RichText::new("설정").color(Color32::WHITE))
                .frame(
                    egui::Frame::window(ctx.style_of(egui::Theme::Dark).as_ref()).fill(BACKGROUND),
                )
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    apply_text_contrast(ui);
                    ui.label("출력 폴더");
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("폴더 선택").color(Color32::WHITE),
                                )
                                .fill(Color32::BLACK),
                            )
                            .clicked()
                            && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        {
                            self.output_folder = Some(folder);
                        }
                        ui.label(
                            self.output_folder
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "선택되지 않음".to_owned()),
                        );
                    });
                    ui.add_space(12.0);
                    ui.label(convert_engine::document::HWP_NOTICE);
                });
            self.settings_open = open;
        }
    }

    fn content(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for category in Category::ALL {
                if category_tab(ui, category, self.category == category) {
                    self.category = category;
                }
            }
        });
        ui.separator();
        ui.add_space(16.0);

        ui.heading(format!("{} 파일", self.category.label()));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let choose_files = ui
                .scope(|ui| {
                    ui.spacing_mut().button_padding = egui::vec2(16.0, 8.0);
                    ui.add(
                        egui::Button::new(egui::RichText::new("파일 선택").color(Color32::BLACK))
                            .fill(Color32::from_gray(220)),
                    )
                })
                .inner
                .clicked();
            if choose_files && let Some(paths) = rfd::FileDialog::new().pick_files() {
                let category = self.category;
                let files = &mut self.files[self.category.index()];
                let mut pending = Vec::new();
                for path in paths {
                    if let Some(file) = files.iter_mut().find(|file| file.path == path) {
                        if matches!(file.status, ConversionStatus::Running) {
                            continue;
                        }
                        file.formats = &[];
                        file.target_extension = None;
                        file.status = ConversionStatus::Ready;
                        pending.push(path);
                    } else {
                        pending.push(path.clone());
                        files.push(SelectedFile {
                            path,
                            target_extension: None,
                            status: ConversionStatus::Ready,
                            formats: &[],
                        });
                    }
                }
                if !pending.is_empty() {
                    let sender = self.format_sender.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        for path in pending {
                            let formats = category.formats_for(&path);
                            let _ = sender.send(FormatResult {
                                category,
                                path,
                                formats,
                            });
                            ctx.request_repaint();
                        }
                    });
                }
            }
            if self.files[self.category.index()].is_empty() {
                ui.label("선택된 파일 없음");
            }
        });

        let index = self.category.index();
        let can_convert = self.output_folder.is_some();
        let mut requested = Vec::new();
        if !self.files[index].is_empty() {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                header_cell(ui, "파일", FILE_COLUMN_WIDTH);
                header_cell(ui, "원본 확장자", EXTENSION_COLUMN_WIDTH);
                header_cell(ui, "변환할 확장자", 150.0);
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 72.0).max(80.0))
                .show(ui, |ui| {
                    for file in &mut self.files[index] {
                        ui.horizontal(|ui| {
                            let name = file
                                .path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let galley = ui.painter().layout(
                                name,
                                egui::TextStyle::Body.resolve(ui.style()),
                                Color32::WHITE,
                                FILE_COLUMN_WIDTH,
                            );
                            let row_height = galley.size().y.max(26.0);
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(FILE_COLUMN_WIDTH, row_height),
                                Sense::hover(),
                            );
                            ui.painter().galley(
                                egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                                galley,
                                Color32::WHITE,
                            );
                            response.on_hover_ui(|ui| {
                                egui::Frame::NONE
                                    .fill(Color32::from_gray(220))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(file.path.display().to_string())
                                                .color(Color32::BLACK),
                                        );
                                    });
                            });
                            let extension = file
                                .path
                                .extension()
                                .map(|ext| ext.to_string_lossy().to_uppercase())
                                .unwrap_or_else(|| "—".to_owned());
                            ui.add_sized(
                                [EXTENSION_COLUMN_WIDTH, 26.0],
                                egui::Label::new(extension),
                            );
                            let changed = ui
                                .scope(|ui| {
                                    ui.visuals_mut().disabled_alpha = 1.0;
                                    let formats = file.formats;
                                    ui.add_enabled_ui(
                                        !matches!(file.status, ConversionStatus::Running)
                                            && !formats.is_empty(),
                                        |ui| {
                                            extension_selector(
                                                ui,
                                                ("file_extension", index, &file.path),
                                                &mut file.target_extension,
                                                formats,
                                            )
                                        },
                                    )
                                    .inner
                                })
                                .inner;
                            if changed {
                                file.status = ConversionStatus::Ready;
                            }
                            let enabled = can_convert
                                && file.target_extension.is_some()
                                && file.target_extension.as_deref().is_some_and(|target| {
                                    file.formats.iter().any(|format| *format == target)
                                })
                                && !matches!(file.status, ConversionStatus::Running);
                            if conversion_button(ui, "변환", enabled)
                                && let Some(target) = file.target_extension.clone()
                            {
                                file.status = ConversionStatus::Running;
                                requested.push((file.path.clone(), target));
                            }
                            match &file.status {
                                ConversionStatus::Ready => {}
                                ConversionStatus::Running => {
                                    ui.label("변환 중");
                                }
                                ConversionStatus::Complete(path) => {
                                    ui.label("완료").on_hover_text(path.display().to_string());
                                }
                                ConversionStatus::Failed(message) => {
                                    ui.label("실패").on_hover_text(message);
                                }
                            }
                        });
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("일괄 확장자 선택");
                let has_supported_files = self.files[index]
                    .iter()
                    .any(|file| !file.formats.is_empty());
                let batch_formats: Vec<_> = self
                    .category
                    .formats()
                    .iter()
                    .copied()
                    .filter(|format| {
                        has_supported_files
                            && self.files[index]
                                .iter()
                                .filter(|file| !file.formats.is_empty())
                                .all(|file| file.formats.contains(format))
                    })
                    .collect();
                if !self.batch_extensions[index]
                    .as_deref()
                    .is_some_and(|selected| batch_formats.contains(&selected))
                {
                    self.batch_extensions[index] = None;
                }
                if ui
                    .scope(|ui| {
                        ui.visuals_mut().disabled_alpha = 1.0;
                        ui.add_enabled_ui(!batch_formats.is_empty(), |ui| {
                            extension_selector(
                                ui,
                                ("batch_extension", index),
                                &mut self.batch_extensions[index],
                                &batch_formats,
                            )
                        })
                        .inner
                    })
                    .inner
                {
                    let selected = self.batch_extensions[index].clone();
                    for file in &mut self.files[index] {
                        if !file
                            .formats
                            .iter()
                            .any(|format| Some(*format) == selected.as_deref())
                        {
                            continue;
                        }
                        if !matches!(file.status, ConversionStatus::Running) {
                            file.target_extension = selected.clone();
                            file.status = ConversionStatus::Ready;
                        }
                    }
                }
                let enabled = can_convert
                    && self.files[index].iter().any(|file| {
                        file.target_extension.as_deref().is_some_and(|target| {
                            file.formats.iter().any(|format| *format == target)
                        }) && !matches!(file.status, ConversionStatus::Running)
                    });
                if conversion_button(ui, "일괄 확장자 변환", enabled) {
                    for file in &mut self.files[index] {
                        if let Some(target) = file.target_extension.clone()
                            && file.formats.iter().any(|format| *format == target)
                            && !matches!(file.status, ConversionStatus::Running)
                        {
                            file.status = ConversionStatus::Running;
                            requested.push((file.path.clone(), target));
                        }
                    }
                }
            });
        }
        self.start_conversions(requested, ui.ctx());
    }

    fn start_conversions(&self, requested: Vec<(PathBuf, String)>, ctx: &egui::Context) {
        let Some(output_folder) = self.output_folder.clone() else {
            return;
        };
        if requested.is_empty() {
            return;
        }
        let sender = self.result_sender.clone();
        let ctx = ctx.clone();
        let category = self.category;
        let engine_category = match category {
            Category::Image => EngineCategory::Image,
            Category::Document => EngineCategory::Document,
            Category::Audio => EngineCategory::Audio,
            Category::Video => EngineCategory::Video,
        };
        std::thread::spawn(move || {
            for (path, target) in requested {
                let result =
                    convert_engine::convert(engine_category, &path, &target, &output_folder)
                        .map_err(|error| error.to_string());
                let _ = sender.send(ConversionResult {
                    category,
                    path,
                    result,
                });
                ctx.request_repaint();
            }
        });
    }

    fn collect_results(&mut self) {
        while let Ok(result) = self.format_receiver.try_recv() {
            if let Some(file) = self.files[result.category.index()]
                .iter_mut()
                .find(|file| file.path == result.path)
            {
                file.formats = result.formats;
                if !file
                    .target_extension
                    .as_deref()
                    .is_some_and(|target| file.formats.iter().any(|format| *format == target))
                {
                    file.target_extension = None;
                }
            }
        }
        while let Ok(result) = self.result_receiver.try_recv() {
            if let Some(file) = self.files[result.category.index()]
                .iter_mut()
                .find(|file| file.path == result.path)
            {
                file.status = match result.result {
                    Ok(path) => ConversionStatus::Complete(path),
                    Err(message) => ConversionStatus::Failed(message),
                };
            }
        }
    }
}

fn header_cell(ui: &mut Ui, label: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        Color32::WHITE,
    );
}

fn apply_text_contrast(ui: &mut Ui) {
    let visuals = ui.visuals_mut();
    visuals.override_text_color = None;
    visuals.widgets.noninteractive.fg_stroke.color = Color32::WHITE;
    let inactive_fill = visuals.widgets.inactive.weak_bg_fill;
    visuals.widgets.inactive.fg_stroke.color = if inactive_fill.r() >= 140 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    for (widget, fill) in [
        (&mut visuals.widgets.hovered, Color32::from_gray(220)),
        (&mut visuals.widgets.active, Color32::from_gray(200)),
        (&mut visuals.widgets.open, Color32::from_gray(220)),
    ] {
        widget.weak_bg_fill = fill;
        widget.bg_fill = fill;
        widget.fg_stroke.color = Color32::BLACK;
    }
}

fn category_tab(ui: &mut Ui, category: Category, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(84.0, 42.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), 0.18);
    let fill = if selected {
        Color32::WHITE
    } else {
        Color32::from_rgb(
            (43.0 + 37.0 * hover) as u8,
            (47.0 + 38.0 * hover) as u8,
            (53.0 + 40.0 * hover) as u8,
        )
    };
    let text = if selected {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if selected && hover > 0.0 {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5 * hover, Color32::from_rgb(112, 116, 122)),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        category.label(),
        egui::TextStyle::Button.resolve(ui.style()),
        text,
    );
    response.clicked()
}

fn extension_selector(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut Option<String>,
    formats: &[&str],
) -> bool {
    let before = selected.clone();
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.override_text_color = None;
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.weak_bg_fill = Color32::from_gray(220);
            widget.fg_stroke.color = Color32::BLACK;
        }
        egui::ComboBox::from_id_salt(id)
            .width(96.0)
            .selected_text(
                egui::RichText::new(selected.as_deref().unwrap_or("선택")).color(Color32::BLACK),
            )
            .show_ui(ui, |ui| {
                let visuals = ui.visuals_mut();
                visuals.override_text_color = None;
                visuals.widgets.noninteractive.fg_stroke.color = Color32::BLACK;
                visuals.widgets.inactive.fg_stroke.color = Color32::BLACK;
                visuals.selection.bg_fill = Color32::from_gray(200);
                visuals.selection.stroke.color = Color32::BLACK;
                ui.set_min_width(208.0);
                egui::Grid::new("extension_grid")
                    .num_columns(3)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        for (index, &format) in formats.iter().enumerate() {
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(64.0), Sense::click());
                            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(4),
                                Color32::from_gray(220),
                            );
                            if response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    CornerRadius::same(4),
                                    Stroke::new(2.0, Color32::BLACK),
                                    StrokeKind::Inside,
                                );
                            }
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                format,
                                egui::TextStyle::Button.resolve(ui.style()),
                                Color32::BLACK,
                            );
                            if response.clicked() {
                                *selected = Some(format.to_owned());
                                ui.close();
                            }
                            if index % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
            });
    });
    *selected != before
}

fn conversion_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.scope(|ui| {
        ui.visuals_mut().disabled_alpha = 1.0;
        ui.add_enabled(
            enabled,
            egui::Button::new(egui::RichText::new(label).color(Color32::BLACK))
                .fill(Color32::from_gray(220)),
        )
    })
    .inner
    .clicked()
}
