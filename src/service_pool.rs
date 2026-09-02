//! Worker-owned managed-service pool state.
//!
//! Process handles remain behind the executor's opaque promotion adapter. This
//! module tracks lifecycle metadata and serial replacement decisions.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceSpec {
    pub(crate) name: String,
    pub(crate) revision: u64,
    pub(crate) signature: String,
    pub(crate) origin_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceState {
    Starting,
    Ready,
    Restarting,
    Stopping,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceEntry {
    pub(crate) name: String,
    pub(crate) instance_id: u64,
    pub(crate) state: ServiceState,
    pub(crate) revision: u64,
    pub(crate) signature: String,
    pub(crate) origin_generation: Option<u64>,
    pending: Option<ServiceSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PoolAction {
    Start { name: String, instance_id: u64 },
    Stop { name: String, instance_id: u64 },
    Probe { name: String, instance_id: u64 },
}

#[derive(Default)]
pub(crate) struct ManagedServicePool {
    next_instance_id: u64,
    entries: BTreeMap<String, ServiceEntry>,
}

impl ManagedServicePool {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&ServiceEntry> {
        self.entries.get(name)
    }

    pub(crate) fn instance_ids(&self) -> Vec<u64> {
        self.entries.values().map(|entry| entry.instance_id).collect()
    }

    pub(crate) fn promote_ready(&mut self, spec: ServiceSpec) -> ServiceEntry {
        let instance_id = self.allocate_instance_id();
        let entry = ServiceEntry {
            name: spec.name.clone(),
            instance_id,
            state: ServiceState::Ready,
            revision: spec.revision,
            signature: spec.signature,
            origin_generation: spec.origin_generation,
            pending: None,
        };
        self.entries.insert(spec.name, entry.clone());
        entry
    }

    /// A generation only acts on explicitly selected services. Omitted names
    /// remain pooled and untouched, including when their signature is equal.
    pub(crate) fn select_generation(&mut self, specs: &[ServiceSpec]) -> Vec<PoolAction> {
        let mut actions = vec![];
        for spec in specs {
            if let Some(entry) = self.entries.get_mut(&spec.name) {
                if entry.state == ServiceState::Stopping {
                    continue;
                }
                let old_instance = entry.instance_id;
                entry.state = ServiceState::Stopping;
                entry.pending = Some(spec.clone());
                actions.push(PoolAction::Stop {
                    name: spec.name.clone(),
                    instance_id: old_instance,
                });
            } else {
                actions.push(self.start_entry(spec.clone()));
            }
        }
        actions
    }

    /// Reload reconciliation deliberately replaces changed services and
    /// removes omitted ones. New services start immediately; replacements
    /// start only after their old process has been reaped.
    pub(crate) fn reconcile_reload(&mut self, specs: &[ServiceSpec]) -> Vec<PoolAction> {
        let desired: BTreeMap<&str, &ServiceSpec> = specs
            .iter()
            .map(|spec| (spec.name.as_str(), spec))
            .collect();
        let mut actions = vec![];
        let names: Vec<String> = self.entries.keys().cloned().collect();
        for name in names {
            let Some(entry) = self.entries.get_mut(&name) else {
                continue;
            };
            let Some(spec) = desired.get(name.as_str()) else {
                entry.state = ServiceState::Stopping;
                entry.pending = None;
                actions.push(PoolAction::Stop {
                    name,
                    instance_id: entry.instance_id,
                });
                continue;
            };
            if entry.state != ServiceState::Stopping
                && (entry.revision != spec.revision || entry.signature != spec.signature)
            {
                entry.state = ServiceState::Stopping;
                entry.pending = Some((*spec).clone());
                actions.push(PoolAction::Stop {
                    name,
                    instance_id: entry.instance_id,
                });
            }
        }
        for spec in specs {
            if !self.entries.contains_key(&spec.name) {
                actions.push(self.start_entry(spec.clone()));
            }
        }
        actions
    }

    pub(crate) fn stopped(&mut self, name: &str, instance_id: u64) -> Option<PoolAction> {
        let entry = self.entries.get(name)?;
        if entry.instance_id != instance_id || entry.state != ServiceState::Stopping {
            return None;
        }
        let pending = self.entries.get_mut(name)?.pending.take();
        let Some(spec) = pending else {
            self.entries.remove(name);
            return None;
        };
        Some(self.replace_entry(spec))
    }

    pub(crate) fn exited(
        &mut self,
        name: &str,
        instance_id: u64,
        deliberate: bool,
    ) -> Option<PoolAction> {
        let entry = self.entries.get_mut(name)?;
        if entry.instance_id != instance_id || entry.state == ServiceState::Stopping {
            return None;
        }
        if deliberate {
            entry.state = ServiceState::Stopped;
            return None;
        }
        entry.state = ServiceState::Restarting;
        Some(PoolAction::Probe {
            name: name.to_owned(),
            instance_id,
        })
    }

    pub(crate) fn probed(&mut self, name: &str, instance_id: u64, success: bool) -> Option<()> {
        let entry = self.entries.get_mut(name)?;
        if entry.instance_id != instance_id || entry.state != ServiceState::Restarting {
            return None;
        }
        entry.state = if success {
            ServiceState::Ready
        } else {
            ServiceState::Failed
        };
        Some(())
    }

    fn allocate_instance_id(&mut self) -> u64 {
        self.next_instance_id += 1;
        self.next_instance_id
    }

    fn start_entry(&mut self, spec: ServiceSpec) -> PoolAction {
        let instance_id = self.allocate_instance_id();
        self.entries.insert(
            spec.name.clone(),
            ServiceEntry {
                name: spec.name.clone(),
                instance_id,
                state: ServiceState::Starting,
                revision: spec.revision,
                signature: spec.signature,
                origin_generation: spec.origin_generation,
                pending: None,
            },
        );
        PoolAction::Start {
            name: spec.name,
            instance_id,
        }
    }

    fn replace_entry(&mut self, spec: ServiceSpec) -> PoolAction {
        let action = self.start_entry(spec);
        action
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolCommand {
    Shutdown,
    Cancel,
    Supersede,
    ReloadReplacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolFact {
    ServiceExited,
    ReadinessTimedOut,
    ReadinessPassed,
    ReadinessFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolDecision {
    Shutdown,
    Cancelled,
    Superseded,
    ReloadReplacement,
    ServiceExited,
    ReadinessTimedOut,
    ReadinessPassed,
    RetryReadiness,
    Noop,
}

#[derive(Default)]
pub(crate) struct PoolCycle {
    commands: Vec<PoolCommand>,
    facts: Vec<PoolFact>,
}

impl PoolCycle {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn command(&mut self, command: PoolCommand) {
        self.commands.push(command);
    }

    pub(crate) fn fact(&mut self, fact: PoolFact) {
        self.facts.push(fact);
    }

    pub(crate) fn resolve(&self) -> PoolDecision {
        for (command, decision) in [
            (PoolCommand::Shutdown, PoolDecision::Shutdown),
            (PoolCommand::Cancel, PoolDecision::Cancelled),
            (PoolCommand::Supersede, PoolDecision::Superseded),
            (PoolCommand::ReloadReplacement, PoolDecision::ReloadReplacement),
        ] {
            if self.commands.contains(&command) {
                return decision;
            }
        }
        if self.facts.contains(&PoolFact::ServiceExited) {
            return PoolDecision::ServiceExited;
        }
        if self.facts.contains(&PoolFact::ReadinessTimedOut) {
            return PoolDecision::ReadinessTimedOut;
        }
        if self.facts.contains(&PoolFact::ReadinessPassed) {
            return PoolDecision::ReadinessPassed;
        }
        if self.facts.contains(&PoolFact::ReadinessFailed) {
            return PoolDecision::RetryReadiness;
        }
        PoolDecision::Noop
    }
}

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
