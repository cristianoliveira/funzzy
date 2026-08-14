// #![feature(plugin)]
// #![plugin(clippy)]
pub mod app;
pub mod arguments;
pub mod awaiting;
pub mod cli;
pub mod cmd;
pub mod config;
pub mod control;
pub mod control_client;
pub mod environment;
pub mod errors;
pub mod executor;
pub mod identity;
pub mod logging;
pub mod output;
pub mod plan;
pub mod process_owner;
pub mod rules;
pub mod snapshot;
pub mod stdout;
pub mod template;
pub mod watch_loop;
pub mod watcher;
pub mod watches;
pub mod workers;
pub mod workflow;
pub mod yaml;
