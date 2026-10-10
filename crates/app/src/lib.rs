//! lazydelta-app internals shared between the binary and tests:
//! project config, thread persistence, the ACP runner, markdown rendering,
//! and the project open/refresh flow.

pub mod config;
pub mod markdown;
pub mod project;
pub mod runner;
pub mod store;
