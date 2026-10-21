//! Basalt 的数据库核心,不依赖 Tauri,可以单独测试。

mod browse;
pub mod config;
pub mod design;
pub mod driver;
pub mod error;
pub mod import;
pub mod llm;
pub mod query;
pub mod secret;
pub mod session;
pub mod sql;
pub mod table;
pub mod table_info;
pub mod table_ops;
pub mod transfer;
pub mod value;

pub use error::{CoreError, CoreResult, FieldError};
