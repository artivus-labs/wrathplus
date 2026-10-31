use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};

pub const BG: Color = Color::from_rgb8(13, 19, 29);
pub const TEXT: Color = Color::from_rgb8(222, 233, 243);
pub const MUTED: Color = Color::from_rgb8(139, 159, 180);
pub const ICE: Color = Color::from_rgb8(133, 191, 222);
pub const GOLD: Color = Color::from_rgb8(223, 185, 109);

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(22, 32, 46))),
        border: Border {
            color: Color::from_rgb8(43, 59, 77),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}
pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb8(17, 25, 37).into()),
        ..Default::default()
    }
}
pub fn nav(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let mut style = button::text(theme, status);
    style.text_color = if active { ICE } else { MUTED };
    if active || matches!(status, button::Status::Hovered) {
        style.background = Some(Color::from_rgb8(31, 46, 64).into());
    }
    style.border.radius = 8.0.into();
    style
}
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.background = Some(
        if matches!(status, button::Status::Hovered) {
            Color::from_rgb8(239, 205, 143)
        } else {
            GOLD
        }
        .into(),
    );
    style.text_color = BG;
    style.border.radius = 8.0.into();
    style
}
