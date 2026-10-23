//! Basalt 的数据库核心,不依赖 Tauri,可以单独测试。

mod browse;
pub mod config;
pub mod design;
pub mod driver;
pub mod error;
pub mod import;
pub mod llm;
pub mod monitor;
pub mod query;
pub mod redis_db;
pub mod secret;
pub mod session;
pub mod sql;
pub mod table;
pub mod table_info;
pub mod table_ops;
pub mod transfer;
pub mod tunnel;
pub mod value;
pub mod workspace;

pub use error::{CoreError, CoreResult, FieldError};
