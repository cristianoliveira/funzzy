//! Shared finite configured-workflow execution.
//!
//! Watch triggers and the local `run` command use this same preparation path:
//! resolve task context, expand templates, execute the plan, and report a
//! combined outcome. Filesystem watching and control IPC stay outside.

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
}

impl WorkflowRunner {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            concurrency,
            Arc::new(|_| {}),
            fail_fast,
            verbose,
        )
        .expect("workflow concurrency must be positive");
        Self {
            root,
            verbose,
            executor,
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
        stdout::verbose(&plan.context_summary(), self.verbose);
        for variable in unknown_variables {
            stdout::warn(&format!("Unknown template variable '{}'.", variable));
        }
        Ok(self.executor.run_to_completion(metadata, plan))
    }
}
