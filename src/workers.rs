use crate::executor::{
    CancelDisposition, Event, EventSink, Executor, Run, RunMetadata, Step, SystemClock,
    SystemProcessRunner,
};
use crate::output::OutputRegistry;
use crate::plan::{ExecutionSignature, RunPlan};
use crate::rules::Rules;
use crate::stdout;
use crate::template::TemplateOptions;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A run requested through the worker's command stream.
struct RunRequest {
    run_id: u64,
    plan: RunPlan,
    trigger: String,
    /// Debounce batch identity, when scheduled from a filesystem batch.
    batch: Option<u64>,
    /// Complete normalized changed-path set of the triggering batch.
    changed: Vec<String>,
    /// Generation identity this request replaces; set when it supersedes an
    /// active run (restart policy), so the relation survives to start.
    predecessor: Option<u64>,
    /// Exact configured target name (TASK-0054); None for fs/init/emit runs.
    target: Option<String>,
    /// Stable execution signature of the resolved plan (TASK-0054).
    execution_signature: Option<ExecutionSignature>,
    /// Per-generation effective concurrency (TASK-0073): Some(1) for a
    /// sequential control run; None keeps the worker's configured bound.
    effective_concurrency: Option<usize>,
    /// Override source label (TASK-0073): "control" when the generation was
    /// explicitly requested sequential over the control socket.
    concurrency_source: Option<&'static str>,
    /// Run-level terminal hooks (TASK-0040).
    hooks: crate::config::RunHooks,
    /// Immutable config revision this request is frozen under (TASK-0089).
    revision: Option<u64>,
    /// Non-secret semantic hash of the frozen config revision.
    revision_hash: Option<String>,
}

/// Result of an exact-generation cancel (TASK-0046): the generation matched
/// (active or queued) and its termination disposition, or nothing matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelResult {
    Cancelled { disposition: CancelDisposition },
    Noop,
}

/// Scheduling, replacement, and cancellation flow through one ordered stream,
/// so a newer run always supersedes queued work and an exact cancel is a
/// compare-and-act on generation identity instead of a race with a separate
/// channel.
enum WorkerCommand {
    Run(RunRequest),
    Cancel {
        /// `Some(id)` cancels only the exact generation; `None` cancels
        /// whatever is active (replacement/shutdown path).
        generation: Option<u64>,
        /// Reply channel for exact cancels, so the server can report whether
        /// the generation actually matched instead of guessing.
        reply: Option<std::sync::mpsc::Sender<CancelResult>>,
    },
}

/// Ordered command queue: at most one queued Run (newest wins), while
/// cancels are applied in order. The condition variable blocks idle consumers
/// without polling.
#[derive(Default)]
struct SchedulerState {
    queue: VecDeque<WorkerCommand>,
    closed: bool,
}

/// Scheduler that reports discarded queued work (contract §1): every
/// generation superseded before spawn gets a terminal Cancelled event with its
/// successor identity, so exact-generation awaits never hang.
struct Scheduler {
    state: Mutex<SchedulerState>,
    ready: Condvar,
    events: Arc<dyn EventSink>,
}

impl Scheduler {
    fn new(events: Arc<dyn EventSink>) -> Self {
        Self {
            state: Mutex::new(SchedulerState::default()),
            ready: Condvar::new(),
            events,
        }
    }

    fn send(&self, command: WorkerCommand) {
        let mut state = self.state.lock().unwrap();
        match command {
            WorkerCommand::Run(new_req) => {
                let new_id = new_req.run_id;
                // A Run subsumes any immediately-preceding cancel-whatever:
                // the run itself replaces active work, so the bare cancel is
                // redundant (preserves the single-slot overwrite behavior).
                while matches!(
                    state.queue.back(),
                    Some(WorkerCommand::Cancel {
                        generation: None,
                        ..
                    })
                ) {
                    state.queue.pop_back();
                }
                if let Some(pos) = state
                    .queue
                    .iter()
                    .rposition(|command| matches!(command, WorkerCommand::Run(_)))
                {
                    let old_id = match &state.queue[pos] {
                        WorkerCommand::Run(req) => req.run_id,
                        _ => unreachable!("rposition matched a Run"),
                    };
                    state.queue[pos] = WorkerCommand::Run(new_req);
                    self.events.emit(Event::Cancelled {
                        run_id: old_id,
                        superseded_by: Some(new_id),
                    });
                } else {
                    state.queue.push_back(WorkerCommand::Run(new_req));
                }
            }
            WorkerCommand::Cancel {
                generation: Some(id),
                reply,
            } => {
                if let Some(pos) = state.queue.iter().position(
                    |command| matches!(command, WorkerCommand::Run(req) if req.run_id == id),
                ) {
                    // The queued generation never spawns: it is cancelled
                    // before spawn, and the requester is told exactly that.
                    state.queue.remove(pos);
                    if let Some(reply) = reply {
                        let _ = reply.send(CancelResult::Cancelled {
                            disposition: CancelDisposition::Graceful,
                        });
                    }
                    self.events.emit(Event::Cancelled {
                        run_id: id,
                        superseded_by: None,
                    });
                } else {
                    state.queue.push_back(WorkerCommand::Cancel {
                        generation: Some(id),
                        reply,
                    });
                }
            }
            WorkerCommand::Cancel {
                generation: None,
                reply,
            } => {
                // cancel-whatever supersedes any queued Run (the original
                // single-slot overwrite behavior), then reaches the consumer
                // to cancel active work.
                while let Some(pos) = state
                    .queue
                    .iter()
                    .position(|command| matches!(command, WorkerCommand::Run(_)))
                {
                    let old_id = match &state.queue[pos] {
                        WorkerCommand::Run(req) => req.run_id,
                        _ => unreachable!("position matched a Run"),
                    };
                    state.queue.remove(pos);
                    self.events.emit(Event::Cancelled {
                        run_id: old_id,
                        superseded_by: None,
                    });
                }
                state.queue.push_back(WorkerCommand::Cancel {
                    generation: None,
                    reply,
                });
            }
        }
        self.ready.notify_one();
    }

    fn try_recv(&self) -> Option<WorkerCommand> {
        self.state.lock().unwrap().queue.pop_front()
    }

    fn receive(&self) -> Option<WorkerCommand> {
        let mut state = self.state.lock().unwrap();
        while state.queue.is_empty() && !state.closed {
            state = self.ready.wait(state).unwrap();
        }
        state.queue.pop_front()
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}

pub struct Worker {
    scheduler: Option<Arc<Scheduler>>,
    next_run_id: AtomicU64,
    root: PathBuf,
    /// Task concurrency bound; part of the execution signature (TASK-0054).
    concurrency: usize,
    /// Fail-fast policy; part of the execution signature (TASK-0054).
    fail_fast: bool,
    /// Run-level terminal hooks (TASK-0040), applied to target runs.
    hooks: crate::config::RunHooks,
    /// Immutable config revision all plans prepared through this worker are
    /// frozen under (TASK-0089). Captured before plan creation; a reload
    /// (TASK-0090) swaps it at the commit boundary. Interior mutability so
    /// the reload transaction can swap without rebuilding the worker.
    revision: std::sync::Mutex<Option<crate::config_revision::ConfigRevision>>,

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    /// Convenience constructor resolving the workspace root from the process
    /// current directory. Keep usage at the outer boundary; prefer
    /// [`Worker::with_root`] so command template preparation does not depend
    /// on hidden process state.
    pub fn new<F>(verbose: bool, fail_fast: bool, on_event: F) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let root = std::env::current_dir().expect("Unable to get current directory");
        Self::with_root(verbose, fail_fast, root, on_event)
    }

    /// Creates a worker that expands command templates against an explicit
    /// workspace root and the host's available parallelism.
    pub fn with_root<F>(verbose: bool, fail_fast: bool, root: PathBuf, on_event: F) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let concurrency = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        Self::with_root_and_concurrency(verbose, fail_fast, root, concurrency, on_event)
    }

    /// Creates a worker with an explicit task-concurrency bound.
    pub fn with_root_and_concurrency<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        concurrency: usize,
        on_event: F,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        Self::with_root_and_concurrency_and_outputs(
            verbose,
            fail_fast,
            root,
            concurrency,
            on_event,
            None,
        )
    }

    /// Like [`Worker::with_root_and_concurrency`], additionally feeding a
    /// retained-output registry (TASK-0045): each task's stdout/stderr is
    /// captured bounded and recorded per generation.
    pub fn with_root_and_concurrency_and_outputs<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        concurrency: usize,
        on_event: F,
        outputs: Option<Arc<OutputRegistry>>,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let events: Arc<dyn EventSink> = Arc::new(move |event: Event| {
            on_event(event);
        });
        let scheduler = Arc::new(Scheduler::new(Arc::clone(&events)));
        let consumer_scheduler = Arc::clone(&scheduler);
        let executor = Executor::with_outputs(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            concurrency,
            events,
            fail_fast,
            verbose,
            outputs,
        )
        .expect("worker concurrency must be positive");

        let consumer = std::thread::spawn(move || {
            let mut active: Option<Run> = None;
            let mut pending: Option<RunRequest> = None;

            loop {
                if active.is_none() {
                    // Promote the newest superseding run, or block on the next
                    // command when idle.
                    if let Some(req) = pending.take() {
                        active = Some(
                            executor.start(
                                RunMetadata::correlated(
                                    req.run_id,
                                    req.trigger.clone(),
                                    req.batch,
                                    req.predecessor,
                                    req.changed.clone(),
                                )
                                .with_duration_profile(
                                    req.target.clone(),
                                    req.execution_signature.clone(),
                                )
                                .with_effective_concurrency(req.effective_concurrency)
                                .with_concurrency_source(req.concurrency_source)
                                .with_hooks(req.hooks.clone())
                                .with_revision(
                                    req.revision.unwrap_or(0),
                                    req.revision_hash.clone().unwrap_or_default(),
                                ),
                                req.plan,
                            ),
                        );
                        continue;
                    }

                    match consumer_scheduler.receive() {
                        Some(WorkerCommand::Run(req)) => {
                            active = Some(
                                executor.start(
                                    RunMetadata::correlated(
                                        req.run_id,
                                        req.trigger.clone(),
                                        req.batch,
                                        req.predecessor,
                                        req.changed.clone(),
                                    )
                                    .with_duration_profile(
                                        req.target.clone(),
                                        req.execution_signature.clone(),
                                    )
                                    .with_effective_concurrency(req.effective_concurrency)
                                    .with_concurrency_source(req.concurrency_source)
                                    .with_hooks(req.hooks.clone())
                                    .with_revision(
                                        req.revision.unwrap_or(0),
                                        req.revision_hash.clone().unwrap_or_default(),
                                    ),
                                    req.plan,
                                ),
                            );
                        }
                        Some(WorkerCommand::Cancel { generation, reply }) => {
                            // No active run: an exact cancel is a no-op unless
                            // a matching queued Run was already handled by
                            // `send`. reply is only present for exact cancels.
                            if generation.is_some() {
                                if let Some(reply) = reply {
                                    let _ = reply.send(CancelResult::Noop);
                                }
                            }
                        }
                        None => break,
                    }
                    continue;
                }

                let step = executor.advance(active.as_mut().expect("active run"));
                match step {
                    Step::Running => match consumer_scheduler.try_recv() {
                        Some(WorkerCommand::Run(req)) => {
                            let mut replaced = active.take().expect("active run");
                            let replaced_id = replaced.run_id();
                            executor.cancel(&mut replaced, Some(req.run_id));
                            let mut superseding = req;
                            superseding.predecessor = Some(replaced_id);
                            pending = Some(superseding);
                            // Burst drain (TASK-0083/0090): newer Runs already
                            // queued behind this one supersede it in the
                            // pending slot before promotion, so a burst
                            // schedules only the newest generation — never a
                            // cascade of one-run-per-drain starts. Cancels
                            // seen here are answered inline (never dropped):
                            // the replaced run is no longer active, and a
                            // cancel of a queued pending run drops it.
                            loop {
                                match consumer_scheduler.try_recv() {
                                    Some(WorkerCommand::Run(later)) => {
                                        pending = Some(later);
                                    }
                                    Some(WorkerCommand::Cancel {
                                        generation: Some(id),
                                        reply,
                                    }) => {
                                        let cancelled_pending =
                                            pending.as_ref().is_some_and(|req| req.run_id == id);
                                        if cancelled_pending {
                                            pending = None;
                                        }
                                        if let Some(reply) = reply {
                                            let _ = reply.send(if cancelled_pending {
                                                CancelResult::Cancelled {
                                                    disposition:
                                                        crate::executor::CancelDisposition::Graceful,
                                                }
                                            } else {
                                                CancelResult::Noop
                                            });
                                        }
                                    }
                                    _ => break,
                                }
                            }
                        }
                        Some(WorkerCommand::Cancel { generation, reply }) => match generation {
                            Some(id) => {
                                if active.as_ref().is_some_and(|run| run.run_id() == id) {
                                    let mut cancelled = active.take().expect("active run");
                                    let disposition = executor.cancel(&mut cancelled, None);
                                    if let Some(reply) = reply {
                                        let _ = reply.send(CancelResult::Cancelled { disposition });
                                    }
                                } else if let Some(reply) = reply {
                                    let _ = reply.send(CancelResult::Noop);
                                }
                            }
                            None => {
                                if let Some(mut cancelled) = active.take() {
                                    executor.cancel(&mut cancelled, None);
                                }
                            }
                        },
                        None => std::thread::sleep(Duration::from_millis(200)),
                    },
                    Step::Finished => {
                        let completed = executor.finish(active.take().expect("active run"));
                        stdout::present_results(
                            completed.results,
                            completed.elapsed,
                            Some(&completed.outcome),
                        );
                    }
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            scheduler: Some(scheduler),
            next_run_id: AtomicU64::new(0),
            root,
            concurrency,
            fail_fast,
            hooks: crate::config::RunHooks::default(),
            revision: std::sync::Mutex::new(None),
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel {
                generation: None,
                reply: None,
            });
        }

        Ok(())
    }

    /// Task concurrency bound the worker executes with; part of the
    /// execution signature (TASK-0054/0055).
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// Fail-fast policy the worker executes with; part of the execution
    /// signature (TASK-0054/0055).
    pub fn fail_fast(&self) -> bool {
        self.fail_fast
    }

    /// Attaches run-level terminal hooks (TASK-0040) applied to target runs.
    pub fn with_hooks(mut self, hooks: crate::config::RunHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Binds the immutable config revision all plans prepared through this
    /// worker are frozen under (TASK-0089, CONFIG-RELOAD-CONTRACT §4).
    pub fn with_revision(self, revision: crate::config_revision::ConfigRevision) -> Self {
        *self.revision.lock().unwrap() = Some(revision);
        self
    }

    /// Swaps the frozen config revision at the reload commit boundary
    /// (TASK-0090). Plans prepared after this call carry the new revision;
    /// active runs keep the revision they started under.
    pub fn set_revision(&self, revision: crate::config_revision::ConfigRevision) {
        *self.revision.lock().unwrap() = Some(revision);
    }

    /// Cancels an exact generation through the worker command stream
    /// (TASK-0046): a compare-and-act on generation identity. Returns whether
    /// the generation matched (active or queued) and how it terminated, or a
    /// no-op when it was already terminal or unknown. Bounded by the shutdown
    /// grace plus a margin; the consumer always replies.
    pub fn cancel_generation(&self, generation: u64) -> Result<CancelResult, String> {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return Err("worker scheduler is unavailable".to_string());
        };
        let (reply, receipt) = std::sync::mpsc::channel();
        scheduler.send(WorkerCommand::Cancel {
            generation: Some(generation),
            reply: Some(reply),
        });
        let (_, grace) = crate::process_owner::shutdown_policy();
        let bound = grace + Duration::from_secs(5);
        receipt
            .recv_timeout(bound)
            .map_err(|_| "cancellation acknowledgement timed out".to_string())
    }

    pub fn schedule(&self, rules: Vec<Rules>, filepath: &str) -> Result<u64, String> {
        self.schedule_plan(RunPlan::from_rules(rules), filepath)
    }

    pub fn schedule_plan(&self, plan: RunPlan, filepath: &str) -> Result<u64, String> {
        self.schedule_plan_with_trigger(plan, filepath, Some(filepath))
    }

    #[cfg(test)]
    pub(crate) fn schedule_with_trigger(
        &self,
        rules: Vec<Rules>,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        self.schedule_plan_with_trigger(RunPlan::from_rules(rules), trigger, filepath)
    }

    pub(crate) fn schedule_plan_with_trigger(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        self.schedule_plan_correlated(plan, trigger, filepath, None, vec![])
    }

    /// Schedules an exact configured target run with its stable execution
    /// signature (TASK-0054). The signature is computed from the resolved
    /// and expanded plan, so cwd/env/topology changes invalidate history
    /// without parsing the trigger string. Filesystem/init/emit runs go
    /// through [`Worker::schedule_plan_correlated`] and carry no signature,
    /// so they never contaminate target history.
    ///
    /// `sequential` (TASK-0073) requests effective concurrency one for this
    /// exact generation; the signature uses the effective concurrency so
    /// sequential duration history cannot contaminate parallel estimates.
    pub(crate) fn schedule_target(
        &self,
        plan: RunPlan,
        target: &str,
        sequential: bool,
    ) -> Result<u64, String> {
        let effective = if sequential { 1 } else { self.concurrency };
        // The trigger label stays `control:<target>` (compatibility surface);
        // profile identity is carried structurally via `target` + signature,
        // never parsed from the trigger string.
        let request =
            self.prepare_request(plan, &format!("control:{}", target), None, None, vec![])?;
        let request = RunRequest {
            target: Some(target.to_owned()),
            execution_signature: Some(request.plan.execution_signature(effective, self.fail_fast)),
            effective_concurrency: Some(effective),
            concurrency_source: sequential.then_some("control"),
            hooks: self.hooks.clone(),
            ..request
        };
        self.dispatch(request)
    }

    /// Schedules a run with its batch correlation (contract §1): the debounce
    /// batch identity and complete changed-path set ride on the generation
    /// from scheduling through start. The predecessor relation is filled by
    /// the consumer when this run supersedes an active one.
    pub(crate) fn schedule_plan_correlated(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
        batch: Option<u64>,
        changed: Vec<String>,
    ) -> Result<u64, String> {
        let request = self.prepare_request(plan, trigger, filepath, batch, changed)?;
        self.dispatch(request)
    }

    /// Resolves and expands a plan against the workspace root, emitting the
    /// same verbose diagnostics as every other scheduling path.
    fn prepare_request(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
        batch: Option<u64>,
        changed: Vec<String>,
    ) -> Result<RunRequest, String> {
        let plan = plan.resolve_context(&self.root)?;
        let (plan, unknown_variables) = plan.expand(&TemplateOptions {
            filepath: filepath.map(str::to_string),
            // TASK-0031: the complete normalized changed-path set rides the
            // generation; expose it as {{paths}} for batch-aware commands.
            paths: changed.clone(),
            current_dir: format!("{}", self.root.display()),
        });
        for variable in unknown_variables {
            stdout::warn(&format!("Unknown template variable '{}'.", variable));
        }
        let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
        let revision = self.revision.lock().unwrap().clone();
        Ok(RunRequest {
            run_id,
            plan,
            trigger: trigger.to_string(),
            batch,
            changed,
            predecessor: None,
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            hooks: crate::config::RunHooks::default(),
            revision: revision.as_ref().map(|r| r.number),
            revision_hash: revision.as_ref().map(|r| r.hash.clone()),
        })
    }

    /// Sends a prepared run request through the scheduler.
    fn dispatch(&self, request: RunRequest) -> Result<u64, String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let run_id = request.run_id;
            scheduler.send(WorkerCommand::Run(request));
            return Ok(run_id);
        }

        Err("worker scheduler is unavailable".to_string())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel {
                generation: None,
                reply: None,
            });
            scheduler.close();
        }
        self.scheduler.take();
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Event as WorkerEvent;
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Instant;

    fn output_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("funzzy-worker-{}-{}", std::process::id(), name))
    }

    fn write_file_rule(path: &PathBuf) -> Rules {
        Rules::new(
            "test".to_string(),
            vec![format!("echo triggered > {}", path.display())],
            vec!["src/**/*.rs".to_string()],
            vec![],
            false,
        )
    }

    fn worker_with_events(verbose: bool, fail_fast: bool) -> (Worker, Receiver<WorkerEvent>) {
        let (tx, rx) = channel();
        (
            Worker::new(verbose, fail_fast, move |event| {
                tx.send(event).unwrap();
            }),
            rx,
        )
    }

    fn rule(commands: Vec<&str>) -> Rules {
        Rules::new(
            "test".to_string(),
            commands.into_iter().map(str::to_string).collect(),
            vec![],
            vec![],
            false,
        )
    }

    fn expect_event<F>(rx: &Receiver<WorkerEvent>, what: &str, pred: F) -> WorkerEvent
    where
        F: Fn(&WorkerEvent) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) if pred(&event) => return event,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        panic!("timed out waiting for {}", what);
    }

    fn collect_until_finished(rx: &Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
        let mut events = vec![];
        loop {
            let event = rx
                .recv_timeout(Duration::from_secs(10))
                .expect("timed out waiting for worker to finish a run");
            let finished = matches!(event, WorkerEvent::Finished { .. });
            events.push(event);
            if finished {
                return events;
            }
        }
    }

    #[test]
    fn burst_replacement_runs_only_the_newest_generation() {
        let (worker, rx) = worker_with_events(false, false);

        let slow = rule(vec!["sleep 5"]);
        let quick = rule(vec!["echo ok"]);

        let first = worker.schedule(vec![slow.clone()], "a.txt").unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        // The consumer is now polling the active child; wait for a tick so both
        // follow-up schedules are queued before the next replacement drain.
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        let second = worker.schedule(vec![slow], "b.txt").unwrap();
        let third = worker.schedule(vec![quick], "c.txt").unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        let started: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                WorkerEvent::Started { run_id, .. } => Some(*run_id),
                _ => None,
            })
            .collect();

        assert_eq!(
            started,
            vec![third],
            "only the newest generation may start after the replacement"
        );
        assert!(
            !started.contains(&second),
            "intermediate generations must be discarded before process spawn"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Cancelled { .. })),
            "the superseded run must be cancelled"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Finished { .. })),
            "the newest generation must finish"
        );
    }

    #[test]
    fn replaced_run_never_executes_remaining_commands() {
        let output = output_file("replaced-remaining");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(false, false);
        let slow = Rules::new(
            "test".to_string(),
            vec![
                "sleep 5".to_string(),
                format!("echo must-not-run > {}", output.display()),
            ],
            vec![],
            vec![],
            false,
        );
        let first = worker.schedule(vec![slow], "a.txt").unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );

        worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);
        drop(worker);

        assert!(
            !output.exists(),
            "superseded run must not execute commands after cancellation"
        );
    }

    #[test]
    fn explicit_cancel_terminates_the_active_run() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();

        expect_event(
            &rx,
            "run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );

        worker.cancel_running_tasks().unwrap();

        expect_event(&rx, "run to be cancelled", |e| {
            matches!(e, WorkerEvent::Cancelled { .. })
        });
        drop(worker);
        assert!(
            matches!(rx.recv_timeout(Duration::from_millis(300)), Err(_)),
            "a cancelled run must never emit Finished"
        );
    }

    #[test]
    fn fail_fast_stops_after_first_failed_command() {
        let output = output_file("fail-fast");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(true, true);
        let commands = vec![
            "false".to_string(),
            format!("echo must-not-run > {}", output.display()),
        ];
        worker
            .schedule(
                vec![Rules::new(
                    "test".to_string(),
                    commands,
                    vec![],
                    vec![],
                    false,
                )],
                "a.txt",
            )
            .unwrap();

        collect_until_finished(&rx);
        drop(worker);

        assert!(
            !output.exists(),
            "fail-fast must skip remaining commands after a failure"
        );
    }

    #[test]
    fn without_fail_fast_later_commands_still_run_after_a_failure() {
        let output = output_file("no-fail-fast");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(false, false);
        let commands = vec![
            "false".to_string(),
            format!("echo ran > {}", output.display()),
        ];
        worker
            .schedule(
                vec![Rules::new(
                    "test".to_string(),
                    commands,
                    vec![],
                    vec![],
                    false,
                )],
                "a.txt",
            )
            .unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        assert!(
            output.exists(),
            "later commands must run when fail-fast is disabled"
        );
        let failures: Vec<String> = match events.last().unwrap() {
            WorkerEvent::Finished { failures, .. } => failures.clone(),
            _ => vec![],
        };
        assert_eq!(failures.len(), 1, "the single failure must be reported");
    }

    #[test]
    fn it_templates_relative_filepath_against_the_injected_root() {
        let marker = output_file("injected-root");
        let _ = std::fs::remove_file(&marker);

        let root =
            std::env::temp_dir().join(format!("funzzy root with spaces {}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create injected root");
        let (tx, rx) = channel();
        let worker = Worker::with_root(false, false, root.clone(), move |event| {
            tx.send(event).unwrap();
        });

        let rule = Rules::new(
            "test".to_string(),
            vec![format!(
                "echo '{{{{relative_filepath}}}}' > {}",
                marker.display()
            )],
            vec![],
            vec![],
            false,
        );
        let filepath = root.join("src/main.rs");
        worker
            .schedule_with_trigger(
                vec![rule],
                filepath.to_str().unwrap(),
                Some(filepath.to_str().unwrap()),
            )
            .unwrap();

        collect_until_finished(&rx);
        drop(worker);

        let content = std::fs::read_to_string(&marker).expect("marker file was not written");
        assert_eq!(
            content.trim(),
            "src/main.rs",
            "template expansion must be relative to the injected root"
        );
        let _ = std::fs::remove_file(&marker);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn it_does_not_run_scheduled_tasks_when_worker_is_dropped() {
        let output = output_file("dropped");
        let _ = std::fs::remove_file(&output);

        {
            let worker = Worker::new(false, false, |_| {});
            let rule = Rules::new(
                "test".to_string(),
                vec![
                    "sleep 1".to_string(),
                    format!("echo triggered > {}", output.display()),
                ],
                vec!["src/**/*.rs".to_string()],
                vec![],
                false,
            );
            worker.schedule(vec![rule], "src/main.rs").unwrap();
        }

        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!output.exists(), "dropped worker should not run hooks");
    }

    #[test]
    fn replacement_records_predecessor_and_superseded_relations() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        let events = collect_until_finished(&rx);
        drop(worker);

        let cancelled = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::Cancelled {
                    run_id,
                    superseded_by,
                } => Some((*run_id, *superseded_by)),
                _ => None,
            })
            .expect("replacement cancellation must be recorded");
        assert_eq!(
            cancelled,
            (first, Some(second)),
            "the superseded generation names its successor"
        );

        let predecessor = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::Started {
                    run_id,
                    predecessor,
                    ..
                } if *run_id == second => Some(*predecessor),
                _ => None,
            })
            .expect("superseding generation must start");
        assert_eq!(
            predecessor,
            Some(first),
            "the superseding generation names its predecessor"
        );
    }

    #[test]
    fn discarded_queued_generation_reports_superseded_terminal() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        // Two rapid schedules: the middle one is discarded from the queue
        // before spawn and must still reach a terminal superseded outcome.
        let middle = worker
            .schedule(vec![rule(vec!["echo mid"])], "b.txt")
            .unwrap();
        let last = worker
            .schedule(vec![rule(vec!["echo last"])], "c.txt")
            .unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        let discarded = events.iter().find_map(|e| match e {
            WorkerEvent::Cancelled {
                run_id,
                superseded_by,
            } => Some((*run_id, *superseded_by)),
            _ => None,
        });
        assert!(
            matches!(discarded, Some((run_id, superseded_by)) if run_id == middle && superseded_by == Some(last)),
            "the discarded queued generation must reach superseded terminal: {events:?}"
        );
    }

    #[test]
    fn cancel_generation_cancels_the_active_run() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );

        let result = worker.cancel_generation(run_id).unwrap();
        assert!(
            matches!(
                result,
                CancelResult::Cancelled {
                    disposition: CancelDisposition::Graceful
                }
            ),
            "expected graceful cancellation, got {result:?}"
        );

        expect_event(&rx, "run to be cancelled", |e| {
            matches!(
                e,
                WorkerEvent::Cancelled {
                    run_id: id,
                    superseded_by: None
                } if *id == run_id
            )
        });
        drop(worker);
    }

    #[test]
    fn cancel_generation_noops_after_terminal() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["echo ok"])], "a.txt")
            .unwrap();
        collect_until_finished(&rx);

        assert_eq!(
            worker.cancel_generation(run_id).unwrap(),
            CancelResult::Noop
        );
        drop(worker);
    }

    #[test]
    fn cancel_generation_noops_for_unknown_generation() {
        let (worker, _rx) = worker_with_events(false, false);
        assert_eq!(worker.cancel_generation(99).unwrap(), CancelResult::Noop);
        drop(worker);
    }

    #[test]
    fn cancel_generation_cancels_a_queued_run_before_spawn() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        let result = worker.cancel_generation(second).unwrap();
        assert!(
            matches!(result, CancelResult::Cancelled { .. }),
            "queued generation must be cancelled, got {result:?}"
        );
        drop(worker);
    }

    #[test]
    fn stale_cancel_does_not_affect_a_newer_generation() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        // Replace first with second, then send a stale cancel for first.
        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);

        // first is now superseded; a cancel for it must be a no-op and must
        // not touch second (already passed).
        assert_eq!(worker.cancel_generation(first).unwrap(), CancelResult::Noop);
        drop(worker);
        let _ = second;
    }

    #[test]
    fn generation_ids_are_never_reused_after_terminal() {
        let (worker, rx) = worker_with_events(false, false);

        let first = worker
            .schedule(vec![rule(vec!["echo ok"])], "a.txt")
            .unwrap();
        collect_until_finished(&rx);

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);
        drop(worker);

        assert!(
            second > first,
            "generation ids must be strictly increasing across terminal outcomes"
        );
    }

    #[test]
    fn it_runs_scheduled_tasks_without_cancel_signal() {
        let output = output_file("scheduled");
        let _ = std::fs::remove_file(&output);

        {
            let worker = Worker::new(false, false, |_| {});
            worker
                .schedule(vec![write_file_rule(&output)], "src/main.rs")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        assert!(output.exists(), "scheduled hook should run");
        let _ = std::fs::remove_file(&output);
    }

    #[test]
    fn target_runs_record_duration_history_through_the_worker_path() {
        use crate::duration_recorder::DurationRecorder;
        use crate::duration_store::DurationStore;
        use crate::plan::RunPlan;

        let temp = std::env::temp_dir().join(format!(
            "funzzy-worker-target-history-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        // Control-run path: schedule_target attaches target + signature
        // structurally; the recorder observes the worker's events and records
        // the terminal wall duration against the profile.
        let store = DurationStore::new(temp.join("run-durations-v1.json"));
        let recorder = Arc::new(DurationRecorder::new(store));
        let recorder_state = Arc::clone(&recorder);
        let worker =
            Worker::with_root_and_concurrency(false, false, temp.clone(), 1, move |event| {
                recorder_state.observe(&event)
            });
        let plan = RunPlan::from_rules(vec![rule(vec!["echo ok"])]);
        // The worker hashes the resolved+expanded plan; the test must match.
        let resolved = plan.resolve_context(&temp).expect("resolve");
        let signature = resolved.execution_signature(1, false);

        worker
            .schedule_target(plan, "build", false)
            .expect("target schedules");
        // Wait until the run reaches terminal (worker polls at 200ms max).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while recorder.success_samples(&signature) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(worker);

        assert_eq!(
            recorder.success_samples(&signature),
            1,
            "control run must record one success sample"
        );
        assert_eq!(recorder.in_flight(), 0, "association removed at terminal");
        let estimate = recorder.estimate(&signature, None).expect("estimate");
        assert_eq!(estimate.samples, 1);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
