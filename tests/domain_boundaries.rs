use funzzy::domain::ports::{
    Clock, EventPublisher, PathObservation, PathObservationSource, PortError, ProcessExecutor,
    ProcessHandle, ProcessStatus,
};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FakeMoment(u64);

struct FakeClock;

impl Clock for FakeClock {
    type Moment = FakeMoment;

    fn now(&self) -> Self::Moment {
        FakeMoment(10)
    }

    fn elapsed_since(&self, earlier: Self::Moment) -> Duration {
        Duration::from_secs(10 - earlier.0)
    }
}

struct FakePathSource {
    next: Option<PathObservation>,
}

impl PathObservationSource for FakePathSource {
    fn next_observation(&mut self) -> Result<Option<PathObservation>, PortError> {
        Ok(self.next.take())
    }
}

struct FakeProcess;

impl ProcessHandle for FakeProcess {
    fn status(&mut self) -> Result<ProcessStatus, PortError> {
        Ok(ProcessStatus::Exited { success: true })
    }

    fn stop(&mut self) -> Result<(), PortError> {
        Ok(())
    }
}

struct FakeProcessExecutor;

impl ProcessExecutor<&'static str> for FakeProcessExecutor {
    type Process = FakeProcess;

    fn start(&self, _request: &'static str) -> Result<Self::Process, PortError> {
        Ok(FakeProcess)
    }
}

#[derive(Default)]
struct RecordingPublisher {
    events: std::sync::Mutex<Vec<&'static str>>,
}

impl EventPublisher<&'static str> for RecordingPublisher {
    fn publish(&self, event: &'static str) -> Result<(), PortError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[test]
fn domain_ports_accept_fakes_without_runtime_adapters() {
    let clock = FakeClock;
    assert_eq!(clock.elapsed_since(clock.now()), Duration::ZERO);

    let mut paths = FakePathSource {
        next: Some(PathObservation::new(
            vec!["src/domain/ports.rs".to_owned()],
            false,
        )),
    };
    assert_eq!(
        paths.next_observation().unwrap(),
        Some(PathObservation::new(
            vec!["src/domain/ports.rs".to_owned()],
            false
        ))
    );

    let mut process = FakeProcessExecutor.start("check").unwrap();
    assert_eq!(
        process.status().unwrap(),
        ProcessStatus::Exited { success: true }
    );
    process.stop().unwrap();

    let publisher = RecordingPublisher::default();
    publisher.publish("finished").unwrap();
    assert_eq!(*publisher.events.lock().unwrap(), ["finished"]);
}

#[test]
fn domain_modules_do_not_import_infrastructure_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let domain_modules = [
        "domain/mod.rs",
        "domain/ports.rs",
        "rules.rs",
        "plan.rs",
        "template.rs",
        "service_lifecycle.rs",
    ];
    let infrastructure_modules = [
        "app",
        "arguments",
        "cli",
        "cmd",
        "config",
        "control",
        "control_client",
        "diagnostics",
        "event_stream",
        "logging",
        "stdout",
        "watch_loop",
        "watcher",
        "watcher_state",
        "workers",
        "workflow",
    ];

    for module in domain_modules {
        let source = std::fs::read_to_string(root.join("src").join(module))
            .unwrap_or_else(|error| panic!("failed to read domain module {module}: {error}"));
        let production_source = source.split("#[cfg(test)]").next().unwrap();
        for infrastructure in infrastructure_modules {
            assert!(
                !production_source.contains(&format!("crate::{infrastructure}")),
                "domain module {module} depends on infrastructure module {infrastructure}"
            );
        }
    }
}
