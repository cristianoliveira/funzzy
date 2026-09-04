//! Domain-facing execution ports.
//!
//! Ports describe observations and capabilities in domain terms. Runtime
//! adapters implement them; process, filesystem, and presentation details do
//! not cross this boundary.

use std::time::{Duration, Instant};

/// Monotonic time and cooperative sleeping used by lifecycle transitions.
///
/// The runtime adapter owns the concrete clock. Tests can provide deterministic
/// implementations without creating a process or consulting wall-clock time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn elapsed(&self, started: Instant) -> Duration;
    fn sleep(&self, duration: Duration);
}
