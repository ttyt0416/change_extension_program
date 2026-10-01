use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, Vec2};

pub(crate) enum Icon {
    ImageEdit,
    Convert,
}

pub(crate) fn draw_image_edit_icon(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let image = Rect::from_center_size(
        Pos2::new(center.x - 3.0, center.y - 3.0),
        Vec2::new(60.0, 48.0),
    );
    let stroke = Stroke::new(2.4, ink);
    painter.rect_stroke(image, CornerRadius::same(5), stroke, StrokeKind::Inside);
    painter.circle_filled(Pos2::new(image.left() + 16.0, image.top() + 14.0), 4.0, ink);
    painter.line_segment(
        [
            Pos2::new(image.left() + 8.0, image.bottom() - 8.0),
            Pos2::new(image.left() + 25.0, image.top() + 24.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(image.left() + 25.0, image.top() + 24.0),
            Pos2::new(image.left() + 37.0, image.bottom() - 8.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x + 10.0, center.y + 26.0),
            Pos2::new(center.x + 33.0, center.y + 3.0),
        ],
        Stroke::new(5.0, fill_icon()),
    );
    painter.line_segment(
        [
            Pos2::new(center.x + 10.0, center.y + 26.0),
            Pos2::new(center.x + 33.0, center.y + 3.0),
        ],
        Stroke::new(2.4, ink),
    );
}

pub(crate) fn draw_convert_icon(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let stroke = Stroke::new(2.4, ink);
    let left = Rect::from_min_size(
        Pos2::new(center.x - 30.0, center.y - 24.0),
        Vec2::new(29.0, 42.0),
    );
    let right = Rect::from_min_size(
        Pos2::new(center.x + 2.0, center.y - 18.0),
        Vec2::new(29.0, 42.0),
    );
    painter.rect_stroke(left, CornerRadius::same(3), stroke, StrokeKind::Inside);
    painter.rect_stroke(right, CornerRadius::same(3), stroke, StrokeKind::Inside);
    painter.line_segment(
        [
            Pos2::new(center.x - 17.0, center.y + 1.0),
            Pos2::new(center.x + 18.0, center.y + 1.0),
        ],
        Stroke::new(5.0, fill_icon()),
    );
    painter.line_segment(
        [
            Pos2::new(center.x - 17.0, center.y + 1.0),
            Pos2::new(center.x + 18.0, center.y + 1.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x + 11.0, center.y - 6.0),
            Pos2::new(center.x + 18.0, center.y + 1.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x + 11.0, center.y + 8.0),
            Pos2::new(center.x + 18.0, center.y + 1.0),
        ],
        stroke,
    );
}

fn fill_icon() -> Color32 {
    Color32::from_rgb(247, 248, 250)
}
