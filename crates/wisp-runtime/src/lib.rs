//! Platform-independent, Wisp pass renderer.
//!
//! Hosts own windows, surfaces and scheduling. This crate owns shader validation,
//! GPU resources and rendering; it has no editor or desktop integration logic.

mod globals;
mod parameters;
mod renderer;
mod shader;
mod texture;

pub use globals::Globals;
pub use renderer::{Gpu, Renderer};
pub use shader::Shader;
pub use texture::WallpaperImage;

mod wallpaper;
pub use wallpaper::{RuntimeLimits, Wallpaper};

pub const EXAMPLE_SHADER: &str = include_str!("../../../examples/shader-basic/wallpaper.wgsl");
pub const EXAMPLE_IMAGE: &[u8] = include_bytes!("../../../examples/shader-basic/background.png");
