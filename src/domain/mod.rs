//! Domain contracts and ports.
//!
//! This module is the dependency boundary for FZZ's domain behavior. Domain
//! planning, matching, lifecycle decisions, generation state, and outcomes may
//! depend on domain values and the ports declared here. They must not depend on
//! CLI parsing, filesystem watcher implementations, child-process APIs,
//! control transport, or terminal/logging adapters.
//!
//! Adapters implement these ports at the application edge. The port traits do
//! not require `Send`, `Sync`, sockets, threads, OS paths, process signals, or
//! concrete serialization formats; those are runtime choices, not domain
//! policy.

pub mod ports;
