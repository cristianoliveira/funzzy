use crate::duration_recorder::DurationRecorder;
use crate::executor::RunMetadata;
use crate::plan::RunPlan;
use crate::stdout;
use crate::workflow::WorkflowRunner;
use std::path::PathBuf;
use std::sync::Arc;

/// Executes one selected configured workflow locally and returns whether its
/// combined task outcome succeeded.
pub struct RunCommand {
    workflow: WorkflowRunner,
}

impl RunCommand {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        Self::with_recorder(root, verbose, fail_fast, concurrency, None)
    }

    /// Creates the local run command with an optional duration recorder
    /// (TASK-0054): exact target runs record their terminal wall duration
    /// against the plan's execution signature, same path as control runs.
    pub fn with_recorder(
        root: PathBuf,
        verbose: bool,
        fail_fast: bool,
        concurrency: usize,
        recorder: Option<Arc<DurationRecorder>>,
    ) -> Self {
        Self::with_recorder_and_events(root, verbose, fail_fast, concurrency, recorder, None)
    }

    /// Creates the local run command with optional duration recorder and
    /// NDJSON run-event stream (TASK-0039).
    pub fn with_recorder_and_events(
        root: PathBuf,
        verbose: bool,
        fail_fast: bool,
        concurrency: usize,
        recorder: Option<Arc<DurationRecorder>>,
        events: Option<Arc<crate::event_stream::EventStream>>,
    ) -> Self {
        Self {
            workflow: WorkflowRunner::with_recorder_and_events(
                root,
                verbose,
                fail_fast,
                concurrency,
                recorder,
                events,
            ),
        }
    }

    /// Attaches run-level terminal hooks (TASK-0040).
    pub fn with_hooks(mut self, hooks: crate::config::GenerationHooks) -> Self {
        self.workflow = self.workflow.with_hooks(hooks);
        self
    }

    pub fn with_recovery_policy(mut self, policy: crate::config::RecoveryPolicy) -> Self {
        self.workflow = self.workflow.with_recovery_policy(policy);
        self
    }

    pub fn with_recovery_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.workflow = self.workflow.with_recovery_timeout(timeout);
        self
    }

    pub fn with_recovery_approval(
        mut self,
        approval: Arc<dyn crate::executor::RecoveryApproval>,
    ) -> Self {
        self.workflow = self.workflow.with_recovery_approval(approval);
        self
    }

    pub fn execute(&self, plan: RunPlan, target: &str) -> Result<bool, String> {
        // Structural target identity (TASK-0054): the recorder never parses
        // the trigger string; the signature is filled from the resolved plan
        // inside the workflow runner.
        let metadata = RunMetadata::new(0, format!("target:{}", target))
            .with_duration_profile(Some(target.to_owned()), None);
        let completed = self.workflow.run(plan, metadata, None)?;
        let succeeded = completed.outcome.is_success();
        stdout::present_results(
            completed.results,
            completed.elapsed,
            Some(&completed.outcome),
            &completed.tasks,
        );
        Ok(succeeded)
    }
}
