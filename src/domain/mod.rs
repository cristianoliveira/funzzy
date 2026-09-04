//! FZZ's internal domain-boundary marker.
//!
//! Domain planning, matching, lifecycle decisions, generation state, and
//! outcomes must not depend on CLI, filesystem watching, process execution,
//! control transport, stdout/logging, or watcher runtime modules.
//!
//! No port API is declared here yet. A port becomes real only when its first
//! domain consumer and adapter land together: configuration validation in
//! TASK-0170, execution transitions in TASK-0171, or control/output queries
//! in TASK-0173. Keeping this module private prevents an unused architecture
//! sketch from becoming a public compatibility surface.

pub(crate) mod finite_lifecycle;
