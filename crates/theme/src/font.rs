use iced::font::{Family, Weight};
use iced::Font;

const GEIST: Family = Family::Name("Geist");
const GEIST_MONO: Family = Family::Name("Geist Mono");

pub const UI: Font = Font { family: GEIST, weight: Weight::Normal, ..Font::DEFAULT };
pub const UI_MEDIUM: Font = Font { family: GEIST, weight: Weight::Medium, ..Font::DEFAULT };
pub const UI_SEMIBOLD: Font = Font { family: GEIST, weight: Weight::Semibold, ..Font::DEFAULT };
pub const UI_BOLD: Font = Font { family: GEIST, weight: Weight::Bold, ..Font::DEFAULT };
pub const MONO: Font = Font { family: GEIST_MONO, weight: Weight::Normal, ..Font::DEFAULT };
pub const MONO_BOLD: Font = Font { family: GEIST_MONO, weight: Weight::Bold, ..Font::DEFAULT };

pub const FONT_BYTES: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Medium.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Bold.ttf"),
];
