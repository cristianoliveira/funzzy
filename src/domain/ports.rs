//! Minimal outbound and inbound contracts for domain behavior.
//!
//! A port carries domain-shaped data only. It deliberately does not expose a
//! filesystem watcher, child-process handle, JSON-RPC request, stdout writer,
//! logger, or thread primitive. Runtime adapters own those details.

use std::fmt;
use std::time::Duration;

/// A stable, displayable failure returned by an adapter.
///
/// The domain can decide whether an adapter failure changes an outcome, while
/// the adapter retains the platform-specific cause in its own layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortError {
    operation: &'static str,
    message: String,
}

impl PortError {
    pub fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }

    pub fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for PortError {}

/// Supplies monotonic time to domain transitions.
///
/// `Moment` remains adapter-defined so the domain never needs a platform clock
/// or a sleeping/threading primitive. Scheduling waits belong to the runtime.
pub trait Clock {
    type Moment: Copy + Eq;

    fn now(&self) -> Self::Moment;
    fn elapsed_since(&self, earlier: Self::Moment) -> Duration;
}

/// A normalized batch of paths observed by an adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathObservation {
    paths: Vec<String>,
    continuous: bool,
}

impl PathObservation {
    pub fn new(paths: Vec<String>, continuous: bool) -> Self {
        Self { paths, continuous }
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn is_continuous(&self) -> bool {
        self.continuous
    }
}

/// Supplies already-normalized path observations to domain planning.
///
/// Registration, recursive watching, debounce implementation, and reload
/// handoff remain filesystem-adapter concerns.
pub trait PathObservationSource {
    fn next_observation(&mut self) -> Result<Option<PathObservation>, PortError>;
}

/// The domain-visible terminal state of a running process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { success: bool },
}

/// A process already started by an execution adapter.
///
/// Signals, process groups, and grace periods are adapter policy. The domain
/// asks only for the semantic operation of stopping a running task.
pub trait ProcessHandle {
    fn status(&mut self) -> Result<ProcessStatus, PortError>;
    fn stop(&mut self) -> Result<(), PortError>;
}

/// Starts a domain-provided request and returns an opaque running process.
///
/// The request type remains generic until the execution transition model is
/// isolated. This prevents this boundary from prematurely copying the current
/// CLI/process command representation.
pub trait ProcessExecutor<Request> {
    type Process: ProcessHandle;

    fn start(&self, request: Request) -> Result<Self::Process, PortError>;
}

/// Publishes a domain event to an adapter.
///
/// Event serialization, retention, stdout rendering, and transport fan-out
/// are adapter concerns. A concrete domain event model will be introduced
/// with the generation-transition extraction.
pub trait EventPublisher<Event> {
    fn publish(&self, event: Event) -> Result<(), PortError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_error_preserves_stable_domain_context() {
        let error = PortError::new("observe paths", "adapter disconnected");

        assert_eq!(error.operation(), "observe paths");
        assert_eq!(error.message(), "adapter disconnected");
        assert_eq!(error.to_string(), "observe paths: adapter disconnected");
    }

    #[test]
    fn path_observation_keeps_paths_and_debounce_fact() {
        let observation = PathObservation::new(vec!["src/rules.rs".to_owned()], true);

        assert_eq!(observation.paths(), ["src/rules.rs"]);
        assert!(observation.is_continuous());
    }
}
