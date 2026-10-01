#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod converter;

use converter::ConverterPage;
use eframe::egui;

pub(crate) const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(43, 47, 53);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("파일 워크스페이스 - 확장자 변환")
            .with_inner_size([720.0, 480.0])
            .with_min_inner_size([440.0, 280.0]),
        ..Default::default()
    };

    eframe::run_native(
        "파일 워크스페이스",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(ConverterPage::default()))
        }),
    )
}

fn install_korean_font(ctx: &egui::Context) {
    let windows_dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let font_path = windows_dir.join("Fonts").join("malgun.ttf");
    if let Ok(bytes) = std::fs::read(font_path) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "malgun".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "malgun".to_owned());
        ctx.set_fonts(fonts);
    }
}
