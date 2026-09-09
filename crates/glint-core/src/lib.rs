pub mod appearance;
pub use appearance::{Appearance, UiSettings};
pub mod model;
pub use model::*;
mod config;
mod matcher;
mod recognizer;
pub use config::ConfigRuntime;
pub use matcher::Matcher;
pub use recognizer::{PrefixTracker, default_templates, recognize};

pub mod logging;
