pub mod groups;
pub mod hierarchy;
pub mod placement;
pub mod render;
pub mod settings;
pub mod system;
pub mod update;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
