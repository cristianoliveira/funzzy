//! Pure policy for selecting finite jobs from one terminal generation.

use crate::task_result::{TaskSnapshot, TaskState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetrySelectionError {
    NoRetryableJobs,
}

/// Selects failed finite jobs in their original declaration order.
///
/// Passed and cancelled jobs are never retried. Services are rejected by the
/// caller after matching these names against the frozen plan.
pub fn retryable_task_names(tasks: &[TaskSnapshot]) -> Result<Vec<String>, RetrySelectionError> {
    let mut eligible = tasks
        .iter()
        .filter(|task| matches!(task.state, TaskState::Failed | TaskState::TimedOut))
        .collect::<Vec<_>>();
    eligible.sort_by_key(|task| task.position);

    if eligible.is_empty() {
        return Err(RetrySelectionError::NoRetryableJobs);
    }

    Ok(eligible.into_iter().map(|task| task.name.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(position: usize, name: &str, state: TaskState) -> TaskSnapshot {
        TaskSnapshot {
            position,
            id: name.to_owned(),
            name: name.to_owned(),
            state,
            duration_ms: None,
        }
    }

    #[test]
    fn selects_failed_and_timed_out_jobs_in_declaration_order() {
        let tasks = vec![
            task(3, "timeout", TaskState::TimedOut),
            task(0, "passed", TaskState::Passed),
            task(2, "failure", TaskState::Failed),
            task(1, "cancelled", TaskState::Cancelled),
        ];

        assert_eq!(
            retryable_task_names(&tasks),
            Ok(vec!["failure".to_owned(), "timeout".to_owned()])
        );
    }

    #[test]
    fn rejects_a_generation_without_failed_or_timed_out_jobs() {
        let tasks = vec![
            task(0, "passed", TaskState::Passed),
            task(1, "cancelled", TaskState::Cancelled),
        ];

        assert_eq!(
            retryable_task_names(&tasks),
            Err(RetrySelectionError::NoRetryableJobs)
        );
    }
}
