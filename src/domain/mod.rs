//! FZZ's internal domain-boundary marker.
//!
//! Domain planning, matching, lifecycle decisions, generation state, and
//! outcomes must not depend on CLI, filesystem watching, process execution,
//! control transport, stdout/logging, or watcher runtime modules.
//!
//! Port APIs become real only when their first domain consumer and adapter
//! land together. TASK-0171 currently owns the lifecycle clock contract;
//! configuration validation is in TASK-0170 and control/output queries remain
//! planned for TASK-0173. Keeping this module private prevents an unused
//! architecture sketch from becoming a public compatibility surface.

pub(crate) mod finite_lifecycle;
pub(crate) mod ports;
