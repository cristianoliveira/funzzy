use crate::executor::RunMetadata;
use crate::plan::RunPlan;
use crate::stdout;
use crate::workflow::WorkflowRunner;
use std::path::PathBuf;

/// Executes one selected configured workflow locally and returns whether its
/// combined task outcome succeeded.
pub struct RunCommand {
    workflow: WorkflowRunner,
}

impl RunCommand {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        Self {
            workflow: WorkflowRunner::new(root, verbose, fail_fast, concurrency),
        }
    }

    pub fn execute(&self, plan: RunPlan, target: &str) -> Result<bool, String> {
        let completed = self.workflow.run(
            plan,
            RunMetadata::new(0, format!("target:{}", target)),
            None,
        )?;
        let succeeded = completed.outcome.is_success();
        stdout::present_results(completed.results, completed.elapsed);
        Ok(succeeded)
    }
}
