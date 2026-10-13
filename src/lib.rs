pub mod adoption;
pub mod agents;
pub mod config;
mod directory_link;
pub mod effective_skills;
pub mod library;
pub mod model;
pub mod project;
pub mod project_discovery;
pub mod proma;
pub mod source;
pub mod tags;
pub mod text;
mod trigger;
mod update_protection;
mod package_fs;

#[cfg(feature = "desktop")]
pub mod assets;
#[cfg(feature = "desktop")]
mod platform;
#[cfg(feature = "desktop")]
pub mod ui;

pub use config::AppConfig;
pub use library::SkillLibrary;
pub use model::*;
