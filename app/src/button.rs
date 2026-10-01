use crate::icons::{Icon, draw_convert_icon, draw_image_edit_icon};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};

pub(crate) fn draw_button(ui: &Ui, rect: Rect, label: &str, icon: Icon) -> bool {
    let response = ui
        .interact(rect, ui.id().with(label), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), 0.18);
    let painter = ui.painter();

    if hover > 0.0 {
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 4.0)),
            CornerRadius::same(14),
            Color32::from_black_alpha((24.0 * hover) as u8),
        );
    }

    let fill = Color32::from_rgb(
        (246.0 - 33.0 * hover) as u8,
        (247.0 - 30.0 * hover) as u8,
        (249.0 - 27.0 * hover) as u8,
    );
    painter.rect_filled(rect, CornerRadius::same(14), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(14),
        Stroke::new(1.5, Color32::from_rgb(188, 194, 201)),
        StrokeKind::Inside,
    );

    let icon_center = Pos2::new(rect.center().x, rect.top() + 70.0);
    let ink = Color32::from_rgb(61, 77, 98);
    match icon {
        Icon::ImageEdit => draw_image_edit_icon(painter, icon_center, ink),
        Icon::Convert => draw_convert_icon(painter, icon_center, ink),
    }

    painter.text(
        Pos2::new(rect.center().x, rect.top() + 139.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(18.0),
        Color32::BLACK,
    );

    response.clicked()
}
