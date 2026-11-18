use iced::font::{Family, Weight};
use iced::Font;

const HUBOT: Family = Family::Name("Hubot Sans");
const JETBRAINS: Family = Family::Name("JetBrains Mono");

pub const UI: Font = Font { family: HUBOT, weight: Weight::Normal, ..Font::DEFAULT };
pub const UI_MEDIUM: Font = Font { family: HUBOT, weight: Weight::Medium, ..Font::DEFAULT };
pub const UI_SEMIBOLD: Font = Font { family: HUBOT, weight: Weight::Semibold, ..Font::DEFAULT };
pub const UI_BOLD: Font = Font { family: HUBOT, weight: Weight::Bold, ..Font::DEFAULT };
pub const MONO: Font = Font { family: JETBRAINS, weight: Weight::Normal, ..Font::DEFAULT };
pub const MONO_BOLD: Font = Font { family: JETBRAINS, weight: Weight::Bold, ..Font::DEFAULT };

pub const FONT_BYTES: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/HubotSans-Regular.ttf"),
    include_bytes!("../../../assets/fonts/HubotSans-Medium.ttf"),
    include_bytes!("../../../assets/fonts/HubotSans-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/HubotSans-Bold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"),
];
