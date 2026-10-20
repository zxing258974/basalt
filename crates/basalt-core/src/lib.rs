//! Basalt 的数据库核心,不依赖 Tauri,可以单独测试。

pub mod config;
pub mod driver;
pub mod error;
pub mod secret;
pub mod session;
pub mod sql;

pub use error::{CoreError, CoreResult, FieldError};
