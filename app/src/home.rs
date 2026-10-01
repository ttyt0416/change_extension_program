use crate::BACKGROUND;
use crate::button::draw_button;
use crate::converter::ConverterPage;
use crate::icons::Icon;
use eframe::egui::{self, Pos2, Rect, Ui, Vec2};

const BUTTON_SIZE: f32 = 184.0;
const BUTTON_GAP: f32 = 24.0;

#[derive(Default)]
pub(crate) struct HomeApp {
    converter_open: bool,
    converter: ConverterPage,
}

impl eframe::App for HomeApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.converter.show_menu_bar(ui);
        if self.converter_open {
            if self.converter.show(ui) {
                self.converter_open = false;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
                    "파일 워크스페이스".to_owned(),
                ));
            }
        } else {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(BACKGROUND))
                .show(ui, |ui| {
                    let center = ui.max_rect().center();
                    let offset = (BUTTON_SIZE + BUTTON_GAP) / 2.0;

                    draw_button(
                        ui,
                        Rect::from_center_size(
                            Pos2::new(center.x - offset, center.y),
                            Vec2::splat(BUTTON_SIZE),
                        ),
                        "이미지 수정",
                        Icon::ImageEdit,
                    );
                    if draw_button(
                        ui,
                        Rect::from_center_size(
                            Pos2::new(center.x + offset, center.y),
                            Vec2::splat(BUTTON_SIZE),
                        ),
                        "확장자 변환",
                        Icon::Convert,
                    ) {
                        self.converter_open = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
                            "파일 워크스페이스 - 확장자 변환".to_owned(),
                        ));
                    }
                });
        }
        self.converter.show_settings_window(ui.ctx());
    }
}
