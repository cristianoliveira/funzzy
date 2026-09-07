//! Worker consumer runtime (TASK-0176).
//!
//! Owns the long-lived consumer loop: active/pending generations, managed
//! service coordination, settled-hook ownership, and command arbitration.
//! Extracted mechanically from the constructor in `super`; dependency
//! construction and thread spawning stay at the construction boundary.

use super::*;

/// Runtime state for the worker consumer loop. The `Worker` handle remains
/// the submission/lifetime surface; this struct is the single owner of the
/// loop-local state (active/pending runs, coordinator, hook owner).
pub(crate) struct WorkerRuntime {
    scheduler: Arc<Scheduler>,
    events: Arc<dyn EventSink>,
    executor: Executor,
    hook_context: crate::plan::TaskContext,
}

impl WorkerRuntime {
    pub(crate) fn new(
        scheduler: Arc<Scheduler>,
        events: Arc<dyn EventSink>,
        executor: Executor,
        hook_context: crate::plan::TaskContext,
    ) -> Self {
        Self {
            scheduler,
            events,
            executor,
            hook_context,
        }
    }

    /// The consumer loop, moved verbatim from the constructor (TASK-0176):
    /// statement order and state representation are unchanged by design;
    /// iteration redesign belongs to TASK-0177.
    pub(crate) fn run(self) {
        let WorkerRuntime {
            scheduler,
            events,
            executor,
            hook_context,
        } = self;
        let mut active: Option<Run> = None;
        let mut pending: Option<RunRequest> = None;
        // Readiness services outlive their settled generation and are
        // owned by this worker until reconciliation or shutdown.
        let mut managed_services = ManagedServiceCoordinator::new();
        let mut settled_hook_owner = SettledHookOwner::new(hook_context.clone());

        loop {
            // Establish the cycle marker before polling pooled children:
            // commands already accepted by the scheduler must be handled
            // first so cancellation/shutdown/reload wins over child facts.
            if !scheduler.has_pending() {
                for service in managed_services.poll(&executor) {
                    if service.state == crate::service_pool::ServiceState::Restarting {
                        stdout::warn(&format!(
                            "service '{}' restarted after an unexpected exit",
                            service.name
                        ));
                    } else if service.state == crate::service_pool::ServiceState::Failed {
                        stdout::warn(&format!("service '{}' failed", service.name));
                    }
                    events.emit(Event::ServiceLifecycle { service });
                }
            }
            if active.is_none() {
                // Promote the newest superseding run, or block on the next
                // command when idle.
                if let Some(req) = pending.take() {
                    managed_services.prepare_run_plan(
                        &executor,
                        &req.plan,
                        req.revision.unwrap_or(0),
                    );
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
                            .with_recovery_policy(req.recovery_policy)
                            .with_recovery_timeout(req.recovery_timeout)
                            .with_hook_context(hook_context.clone())
                            .with_revision(
                                req.revision.unwrap_or(0),
                                req.revision_hash.clone().unwrap_or_default(),
                            ),
                            req.plan,
                        ),
                    );
                    if let Some(run) = active.as_ref() {
                        scheduler.register_active(run.run_id(), run.cancellation_token());
                    }
                    continue;
                }

                match scheduler.receive_until_deadline(Duration::from_millis(200)) {
                    SchedulerWake::Command(command) => {
                        match command {
                            WorkerCommand::Run(req) => {
                                // A new generation supersedes any settled hook;
                                // service-only reconciliation does not.
                                settled_hook_owner.shutdown(&executor);
                                scheduler.cancel_settlement();
                                managed_services.prepare_run_plan(
                                    &executor,
                                    &req.plan,
                                    req.revision.unwrap_or(0),
                                );
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
                                        .with_recovery_policy(req.recovery_policy)
                                        .with_recovery_timeout(req.recovery_timeout)
                                        .with_hook_context(hook_context.clone())
                                        .with_revision(
                                            req.revision.unwrap_or(0),
                                            req.revision_hash.clone().unwrap_or_default(),
                                        ),
                                        req.plan,
                                    ),
                                );
                                if let Some(run) = active.as_ref() {
                                    scheduler
                                        .register_active(run.run_id(), run.cancellation_token());
                                }
                            }
                            WorkerCommand::Cancel { generation, reply } => {
                                if generation.is_none() {
                                    managed_services.cancel_replacements();
                                    settled_hook_owner.shutdown(&executor);
                                    scheduler.cancel_settlement();
                                }
                                // No active run: an exact cancel is a no-op unless
                                // a matching queued Run was already handled by
                                // `send`. reply is only present for exact cancels.
                                if generation.is_some() {
                                    if let Some(reply) = reply {
                                        let _ = reply.send(CancelResult::Noop);
                                    }
                                }
                            }
                            WorkerCommand::ReconcileServices { stop_names, reply } => {
                                let stopped = managed_services.stop_named(&executor, &stop_names);
                                let _ = reply.send(stopped);
                            }
                            WorkerCommand::ReserveServiceReplacement {
                                name,
                                revision,
                                signature,
                                reply,
                            } => {
                                let spec = crate::service_pool::ServiceSpec {
                                    name,
                                    revision: revision.number,
                                    signature,
                                    origin_generation: None,
                                };
                                let action =
                                    managed_services.reserve_service_replacement(&executor, spec);
                                let _ = reply.send(action);
                                emit_service_snapshots(&mut managed_services, &events);
                            }
                            WorkerCommand::AuthorizeServiceReplacement {
                                name,
                                instance_id,
                                reply,
                            } => {
                                let _ = reply.send(
                                    managed_services
                                        .authorize_service_replacement(&name, instance_id),
                                );
                            }
                            WorkerCommand::ReconcileServicePool {
                                desired,
                                revision,
                                reply,
                            } => {
                                let actions = managed_services.reconcile_reload_with_executor(
                                    &executor,
                                    &desired,
                                    revision.number,
                                );
                                let _ = reply.send(actions);
                                emit_service_snapshots(&mut managed_services, &events);
                            }
                            WorkerCommand::ShutdownServicePool { reply } => {
                                managed_services.shutdown(&executor);
                                let _ = reply.send(());
                            }
                            WorkerCommand::StartServices {
                                run_id,
                                plan,
                                revision,
                            } => {
                                // A service-only generation also supersedes
                                // any settled hook when the worker is idle.
                                settled_hook_owner.shutdown(&executor);
                                scheduler.cancel_settlement();
                                // No active generation: start the service plan as
                                // its own generation (services keep it alive).
                                active = Some(
                                    executor.start(
                                        RunMetadata::correlated(
                                            run_id,
                                            "reload:services".to_owned(),
                                            None,
                                            None,
                                            vec![],
                                        )
                                        .with_hook_context(hook_context.clone())
                                        .with_revision(
                                            revision.as_ref().map(|r| r.number).unwrap_or(0),
                                            revision
                                                .as_ref()
                                                .map(|r| r.hash.clone())
                                                .unwrap_or_default(),
                                        ),
                                        plan,
                                    ),
                                );
                                if let Some(run) = active.as_ref() {
                                    scheduler
                                        .register_active(run.run_id(), run.cancellation_token());
                                }
                            }
                        }
                    }
                    SchedulerWake::SettlementDue(spec, token) => {
                        match settled_hook_owner.start(&executor, spec, token) {
                            Ok(true) => {}
                            Ok(false) => scheduler.complete_settlement(),
                            Err(error) => {
                                stdout::warn(&error);
                                scheduler.complete_settlement();
                            }
                        }
                    }
                    SchedulerWake::Timeout => {
                        if let Some((mut run, token)) = settled_hook_owner.running.take() {
                            match Executor::poll_settled_hook(&mut run) {
                                Ok(Some(status)) => {
                                    if !status.success() {
                                        stdout::warn(&format!(
                                            "settled failure hook for generation {} failed with {}",
                                            run.spec.run_id, status
                                        ));
                                    }
                                    scheduler.complete_settlement();
                                }
                                Ok(None) => {
                                    settled_hook_owner.running = Some((run, token));
                                }
                                Err(error) => {
                                    stdout::warn(&error);
                                    scheduler.complete_settlement();
                                }
                            }
                        }
                    }
                    SchedulerWake::Closed => {
                        settled_hook_owner.shutdown(&executor);
                        managed_services.shutdown(&executor);
                        break;
                    }
                }
                continue;
            }

            let step = executor.advance(active.as_mut().expect("active run"));
            match step {
                Step::Running => match scheduler.try_recv() {
                    Some(WorkerCommand::Run(req)) => {
                        let mut replaced = active.take().expect("active run");
                        let replaced_id = replaced.run_id();
                        scheduler.unregister_active(replaced_id);
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
                            match scheduler.try_recv() {
                                Some(WorkerCommand::Run(later)) => {
                                    pending = Some(later);
                                }
                                Some(WorkerCommand::Cancel {
                                    generation: Some(id),
                                    reply,
                                }) => {
                                    let cancelled_pending =
                                        pending.as_ref().is_some_and(|req| req.run_id == id);
                                    let pending_revision = pending
                                        .as_ref()
                                        .filter(|req| req.run_id == id)
                                        .map(|req| (req.revision, req.revision_hash.clone()));
                                    if cancelled_pending {
                                        pending = None;
                                    }
                                    if let Some(reply) = reply {
                                        let _ = reply.send(if cancelled_pending {
                                            CancelResult::Cancelled {
                                                disposition:
                                                    crate::executor::CancelDisposition::Graceful,
                                                revision: pending_revision
                                                    .as_ref()
                                                    .and_then(|(r, _)| *r),
                                                revision_hash: pending_revision
                                                    .as_ref()
                                                    .and_then(|(_, h)| h.clone()),
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
                    Some(WorkerCommand::ReserveServiceReplacement {
                        name,
                        revision,
                        signature,
                        reply,
                    }) => {
                        let action = managed_services.reserve_service_replacement(
                            &executor,
                            crate::service_pool::ServiceSpec {
                                name,
                                revision: revision.number,
                                signature,
                                origin_generation: None,
                            },
                        );
                        let _ = reply.send(action);
                        emit_service_snapshots(&mut managed_services, &events);
                    }
                    Some(WorkerCommand::AuthorizeServiceReplacement {
                        name,
                        instance_id,
                        reply,
                    }) => {
                        let _ = reply.send(
                            managed_services.authorize_service_replacement(&name, instance_id),
                        );
                    }
                    Some(WorkerCommand::ReconcileServicePool {
                        desired,
                        revision,
                        reply,
                    }) => {
                        let actions = managed_services.reconcile_reload_with_executor(
                            &executor,
                            &desired,
                            revision.number,
                        );
                        let _ = reply.send(actions);
                        emit_service_snapshots(&mut managed_services, &events);
                    }
                    Some(WorkerCommand::ShutdownServicePool { reply }) => {
                        managed_services.shutdown(&executor);
                        let _ = reply.send(());
                    }
                    Some(WorkerCommand::ReconcileServices { stop_names, reply }) => {
                        // TASK-0090 AC6: stop the named changed/removed
                        // services owned by the active generation; the
                        // reply names the services still running (the
                        // reload starts new/changed ones under the new
                        // revision). No generation replacement happens, so
                        // active finite work and unchanged services are
                        // untouched (contract §4).
                        if let Some(active) = active.as_mut() {
                            let stop: Vec<&str> = stop_names.iter().map(String::as_str).collect();
                            let still = executor.reconcile_services(active, &stop);
                            let _ = reply.send(still);
                        } else {
                            let stopped = managed_services.stop_named(&executor, &stop_names);
                            let _ = reply.send(stopped);
                        }
                    }
                    Some(WorkerCommand::StartServices {
                        run_id,
                        plan,
                        revision,
                    }) => {
                        // Append the service-only plan to the ACTIVE
                        // generation: new/changed services start under the
                        // committed revision while active finite work and
                        // unchanged services stay owned (contract §4).
                        if let Some(active) = active.as_mut() {
                            executor.append_plan(active, plan);
                        } else {
                            active = Some(
                                executor.start(
                                    RunMetadata::correlated(
                                        run_id,
                                        "reload:services".to_owned(),
                                        None,
                                        None,
                                        vec![],
                                    )
                                    .with_hook_context(hook_context.clone())
                                    .with_revision(
                                        revision.as_ref().map(|r| r.number).unwrap_or(0),
                                        revision
                                            .as_ref()
                                            .map(|r| r.hash.clone())
                                            .unwrap_or_default(),
                                    ),
                                    plan,
                                ),
                            );
                        }
                    }
                    Some(WorkerCommand::Cancel { generation, reply }) => match generation {
                        Some(id) => {
                            if active.as_ref().is_some_and(|run| run.run_id() == id) {
                                let mut cancelled = active.take().expect("active run");
                                scheduler.unregister_active(id);
                                let disposition = executor.cancel(&mut cancelled, None);
                                let revision = cancelled.revision();
                                if let Some(reply) = reply {
                                    let _ = reply.send(CancelResult::Cancelled {
                                        disposition,
                                        revision: revision.as_ref().map(|r| r.number),
                                        revision_hash: revision.as_ref().map(|r| r.hash.clone()),
                                    });
                                }
                            } else if let Some(reply) = reply {
                                let _ = reply.send(CancelResult::Noop);
                            }
                        }
                        None => {
                            managed_services.cancel_replacements();
                            if let Some(mut cancelled) = active.take() {
                                scheduler.unregister_active(cancelled.run_id());
                                executor.cancel(&mut cancelled, None);
                            }
                        }
                    },
                    None => std::thread::sleep(Duration::from_millis(200)),
                },
                Step::Finished => {
                    let completed_run = active.take().expect("active run");
                    scheduler.unregister_active(completed_run.run_id());
                    let completed = executor.finish_with_service_handoff(
                        completed_run,
                        |pending| {
                            if let Some(spec) = pending {
                                scheduler.register_settlement(spec.clone(), Instant::now());
                            }
                        },
                        |services| {
                            managed_services.adopt_handoff(services);
                            for service in managed_services.snapshots() {
                                events.emit(Event::ServiceLifecycle { service });
                            }
                        },
                    );
                    stdout::present_results(
                        completed.results,
                        completed.elapsed,
                        Some(&completed.outcome),
                        &completed.tasks,
                    );
                }
            }
        }

        stdout::info("Consumer thread finished.");
    }
}
