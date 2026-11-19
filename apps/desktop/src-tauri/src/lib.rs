pub mod app;
pub mod boat;
pub mod machine;
pub mod mcp;
pub mod process;
pub mod projects;
pub mod proxy;
pub mod store;
pub mod t3desktop;
pub mod web;
#[cfg(feature = "desktop")]
mod tray;

#[cfg(feature = "desktop")]
pub use tray::run;
