//! Shared finite configured-workflow execution.
//!
//! Watch triggers and the local `run` command use this same preparation path:
//! resolve task context, expand templates, execute the plan, and report a
//! combined outcome. Filesystem watching and control IPC stay outside.

use crate::diagnostics;
use crate::duration_recorder::DurationRecorder;
use crate::executor::{CompletedRun, Executor, RunMetadata, SystemClock, SystemProcessRunner};
use crate::plan::RunPlan;
use crate::stdout;
use crate::template::TemplateOptions;
use std::path::PathBuf;
use std::sync::Arc;

pub struct WorkflowRunner {
    root: PathBuf,
    verbose: bool,
    executor: Executor,
    concurrency: usize,
    fail_fast: bool,
}

impl WorkflowRunner {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        Self::with_recorder(root, verbose, fail_fast, concurrency, None)
    }

    /// Creates a runner that also records exact target runs into duration
    /// history (TASK-0054). The executor event sink forwards to the recorder;
    /// filesystem/init runs carry no target and are ignored by the recorder.
    pub fn with_recorder(
        root: PathBuf,
        verbose: bool,
        fail_fast: bool,
        concurrency: usize,
        recorder: Option<Arc<DurationRecorder>>,
    ) -> Self {
        let events: Arc<dyn crate::executor::EventSink> = match &recorder {
            Some(recorder) => {
                let recorder = Arc::clone(recorder);
                Arc::new(move |event| recorder.observe(&event))
            }
            None => Arc::new(|_| {}),
        };
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            concurrency,
            events,
            fail_fast,
            verbose,
        )
        .expect("workflow concurrency must be positive");
        Self {
            root,
            verbose,
            executor,
            concurrency,
            fail_fast,
        }
    }

    pub fn run(
        &self,
        plan: RunPlan,
        metadata: RunMetadata,
        filepath: Option<&str>,
    ) -> Result<CompletedRun, String> {
        let plan = plan.resolve_context(&self.root)?;
        let (plan, unknown_variables) = plan.expand(&TemplateOptions {
            filepath: filepath.map(str::to_owned),
            current_dir: self.root.display().to_string(),
        });
        if self.verbose {
            // Blocking strategy: one in-process run per decision; the record
            // carries the debounce batch when scheduled from a file change.
            diagnostics::debug(&diagnostics::Record {
                batch: metadata.batch,
                source: if metadata.target.is_some() {
                    Some("control")
                } else if metadata.batch.is_some() {
                    Some("filesystem")
                } else {
                    Some("init")
                },
                decision: Some("scheduled"),
                generation: Some(metadata.run_id),
                policy: Some("wait"),
                commands: Some(plan.commands().len()),
                ..Default::default()
            });
        }
        for variable in unknown_variables {
            stdout::warn(&format!("Unknown template variable '{}'.", variable));
        }
        // Exact target runs carry their profile identity structurally
        // (TASK-0054): the signature is derived from the resolved+expanded
        // plan, never parsed from the trigger string.
        let mut metadata = metadata;
        if metadata.target.is_some() && metadata.execution_signature.is_none() {
            metadata.execution_signature =
                Some(plan.execution_signature(self.concurrency, self.fail_fast));
        }
        Ok(self.executor.run_to_completion(metadata, plan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration_recorder::DurationRecorder;
    use crate::duration_store::DurationStore;
    use crate::executor::RunMetadata;
    use crate::plan::RunPlan;
    use crate::rules::Rules;

    fn target_rule(name: &str) -> Rules {
        Rules::new(
            name.to_owned(),
            vec!["true".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
    }

    #[test]
    fn local_target_run_records_duration_history() {
        // Local `fzz run TARGET` path: RunMetadata carries the target
        // structurally; the runner computes the signature from the resolved
        // plan and the recorder (fed by the executor sink) records the
        // terminal wall duration — same recording path as control runs.
        let temp = std::env::temp_dir().join(format!(
            "funzzy-workflow-target-history-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        let store = DurationStore::new(temp.join("run-durations-v1.json"));
        let recorder = Arc::new(DurationRecorder::new(store));
        let runner = WorkflowRunner::with_recorder(
            temp.clone(),
            false,
            false,
            1,
            Some(Arc::clone(&recorder)),
        );

        let plan = RunPlan::from_rules(vec![target_rule("build")]);
        let resolved = plan.resolve_context(&temp).expect("resolve");
        let signature = resolved.execution_signature(1, false);

        let metadata = RunMetadata::new(0, "target:build")
            .with_duration_profile(Some("build".to_owned()), None);
        let completed = runner
            .run(plan, metadata, None)
            .expect("target runs to completion");
        assert!(completed.outcome.is_success());

        assert_eq!(
            recorder.success_samples(&signature),
            1,
            "local run must record one success sample"
        );
        assert_eq!(recorder.in_flight(), 0);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn filesystem_run_without_target_is_not_recorded() {
        let temp = std::env::temp_dir().join(format!(
            "funzzy-workflow-fs-not-recorded-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        let store = DurationStore::new(temp.join("run-durations-v1.json"));
        let recorder = Arc::new(DurationRecorder::new(store));
        let runner = WorkflowRunner::with_recorder(
            temp.clone(),
            false,
            false,
            1,
            Some(Arc::clone(&recorder)),
        );

        let plan = RunPlan::from_rules(vec![target_rule("build")]);
        let metadata = RunMetadata::new(0, "init"); // no target: fs/init run
        let completed = runner.run(plan, metadata, None).expect("run");
        assert!(completed.outcome.is_success());
        assert_eq!(recorder.in_flight(), 0);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
