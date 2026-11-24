use rune::Color;
use rune::color::hex;
use rune::tok::{Family, TextStyle};

pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];

pub const FAMILIES: (&str, &str) = ("IBM Plex Sans", "IBM Plex Mono");

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub panel: Color,
    pub raised: Color,
    pub text: Color,
    pub muted: Color,
    pub line: Color,
    pub hover: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub selected: Color,
    pub green: Color,
    pub red: Color,
    pub added: Color,
    pub removed: Color,
}

fn color(value: &str) -> Color {
    hex(value).expect("palette colors are valid hex literals")
}

impl Palette {
    pub fn of(dark: bool) -> Self {
        if dark {
            Self {
                bg: color("#132125"),
                panel: color("#0d191d"),
                raised: color("#192b30"),
                text: color("#e2edeb"),
                muted: color("#a2b8ba"),
                line: color("#2b4145"),
                hover: color("#20383d"),
                accent: color("#a0d7de"),
                on_accent: color("#102e37"),
                selected: color("#26434a"),
                green: color("#a5ca96"),
                red: color("#ecad9d"),
                added: color("#22392d"),
                removed: color("#3f302f"),
            }
        } else {
            Self {
                bg: color("#fafcfb"),
                panel: color("#eef2f0"),
                raised: color("#ffffff"),
                text: color("#162f36"),
                muted: color("#52696e"),
                line: color("#cddbd7"),
                hover: color("#e3ece8"),
                accent: color("#075d70"),
                on_accent: color("#ffffff"),
                selected: color("#dcebec"),
                green: color("#326a48"),
                red: color("#9e4241"),
                added: color("#e5eee5"),
                removed: color("#f3e5e1"),
            }
        }
    }
}

pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
pub const BODY_MEDIUM: TextStyle = TextStyle::new(13.0, 500.0);
pub const BODY_STRONG: TextStyle = TextStyle::new(13.0, 600.0);
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
pub const TINY: TextStyle = TextStyle::new(11.0, 400.0);
pub const TITLE: TextStyle = TextStyle::new(20.0, 600.0);
pub const HEADING: TextStyle = TextStyle::new(16.0, 600.0);
pub const WORDMARK: TextStyle = TextStyle::new(24.0, 400.0);
pub const WORDMARK_BOLD: TextStyle = TextStyle::new(24.0, 600.0);

pub const fn mono(size: f32, weight: f32, line_height: f32) -> TextStyle {
    TextStyle {
        size,
        weight,
        family: Family::Mono,
        line_height,
        tracking: 0.0,
        variations: "",
        features: "",
        optical_size: rune::tok::OpticalSize::Auto,
        italic: false,
    }
}

pub const CODE: TextStyle = mono(12.0, 400.0, 25.0);
pub const TERMINAL: TextStyle = mono(12.0, 400.0, 16.0);
pub const PATH: TextStyle = mono(12.0, 500.0, 18.0);
pub const PATH_SMALL: TextStyle = mono(11.0, 400.0, 16.0);
