//! Worker-owned managed-service pool state.
//!
//! Process handles remain behind the executor's opaque promotion adapter. This
//! module tracks only lifecycle metadata and serial replacement decisions.

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, revision: u64, signature: &str) -> ServiceSpec {
        ServiceSpec {
            name: name.to_owned(),
            revision,
            signature: signature.to_owned(),
            origin_generation: Some(10),
        }
    }

    #[test]
    fn promotion_assigns_monotonic_instance_and_ready_state() {
        let mut pool = ManagedServicePool::new();
        let entry = pool.promote_ready(spec("api", 1, "sha256:a"));
        assert_eq!(entry.instance_id, 1);
        assert_eq!(entry.state, ServiceState::Ready);
        assert_eq!(pool.get("api").unwrap().instance_id, 1);
    }

    #[test]
    fn generation_omission_keeps_an_existing_ready_service_running() {
        let mut pool = ManagedServicePool::new();
        pool.promote_ready(spec("api", 1, "sha256:a"));
        assert!(pool.select_generation(&[]).is_empty());
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Ready);
    }

    #[test]
    fn generation_reinclusion_reserves_then_stops_before_starting_replacement() {
        let mut pool = ManagedServicePool::new();
        pool.promote_ready(spec("api", 1, "sha256:a"));
        let actions = pool.select_generation(&[spec("api", 2, "sha256:a")]);
        assert_eq!(actions, vec![PoolAction::Stop { name: "api".into(), instance_id: 1 }]);
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Stopping);

        let action = pool.stopped("api", 1).expect("replacement starts after reap");
        assert_eq!(action, PoolAction::Start { name: "api".into(), instance_id: 2 });
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Starting);
        assert_eq!(pool.instance_ids(), vec![2]);
    }

    #[test]
    fn reload_add_change_remove_has_deterministic_actions() {
        let mut pool = ManagedServicePool::new();
        pool.promote_ready(spec("api", 1, "sha256:a"));
        let actions = pool.reconcile_reload(&[spec("db", 1, "sha256:d"), spec("api", 2, "sha256:b")]);
        assert_eq!(
            actions,
            vec![
                PoolAction::Stop { name: "api".into(), instance_id: 1 },
                PoolAction::Start { name: "db".into(), instance_id: 2 },
            ]
        );
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Stopping);
        assert_eq!(pool.get("db").unwrap().state, ServiceState::Starting);
    }

    #[test]
    fn post_settlement_exit_enters_restart_and_probe_success_returns_ready() {
        let mut pool = ManagedServicePool::new();
        pool.promote_ready(spec("api", 1, "sha256:a"));
        assert_eq!(pool.exited("api", 1, false), Some(PoolAction::Probe { name: "api".into(), instance_id: 1 }));
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Restarting);
        pool.probed("api", 1, true).expect("probe belongs to restart");
        assert_eq!(pool.get("api").unwrap().state, ServiceState::Ready);
    }

    #[test]
    fn cycle_commands_have_stable_priority_over_child_facts() {
        let mut cycle = PoolCycle::new();
        cycle.command(PoolCommand::ReloadReplacement);
        cycle.command(PoolCommand::Shutdown);
        cycle.fact(PoolFact::ReadinessPassed);
        assert_eq!(cycle.resolve(), PoolDecision::Shutdown);
    }
}
