pub(crate) mod black_frame;
pub mod gamut;
pub(crate) mod sampling;
pub mod zones;

pub use gamut::HueGamut;
pub use zones::{ActiveRect, ColorProcessor, RgbColor, ZoneSampler};
