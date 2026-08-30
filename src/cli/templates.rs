//! One typed template/profile selector shared by `fzz init --template` and
//! `fzz config example` (TASK-0097). Destination policy (file vs stdout)
//! lives in the command layer; generated YAML bytes live here so the two
//! commands can never drift apart.
//!
//! The comprehensive template stays catalog-driven (`init::render_init_template`,
//! INIT-TEMPLATE-CONTRACT); the named runnable profiles are owned by this
//! module and are the single constants consumed by both commands.

use crate::errors::FzzError;

/// Template profiles accepted by `fzz init --template` and
/// `fzz config example` (AGENT-CONFIG-CONTRACT §4, CLI-V2-CONTRACT §3a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The full commented starter `fzz init` writes by default.
    Comprehensive,
    /// One job, one change pattern — the smallest runnable config.
    Minimal,
    /// Two jobs in one parallel group — demonstrates barriers.
    Parallel,
    /// Control socket + a verify-style job — the agent loop starting point.
    Agent,
}

impl Profile {
    /// Stable names in help/validation order; clap possible values derive
    /// from this list so the typed selector is the single source.
    pub const NAMES: [&'static str; 4] = ["comprehensive", "minimal", "parallel", "agent"];

    /// Parses one accepted profile name; unknown names are `None` so callers
    /// (and clap) own error presentation.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "comprehensive" => Some(Profile::Comprehensive),
            "minimal" => Some(Profile::Minimal),
            "parallel" => Some(Profile::Parallel),
            "agent" => Some(Profile::Agent),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Profile::Comprehensive => "comprehensive",
            Profile::Minimal => "minimal",
            Profile::Parallel => "parallel",
            Profile::Agent => "agent",
        }
    }

    /// The one renderer: bytes are identical regardless of destination
    /// (`fzz init --template P` writes exactly what `fzz config example P`
    /// prints). Every artifact parses and validates through the production
    /// parser (tested below).
    pub fn render(&self) -> String {
        match self {
            Profile::Comprehensive => crate::cli::init::render_init_template(),
            Profile::Minimal => minimal_yaml().to_owned(),
            Profile::Parallel => parallel_yaml().to_owned(),
            Profile::Agent => agent_yaml().to_owned(),
        }
    }
}

/// Renders by name; used by `config example` dispatch where clap already
/// validated the value. Unknown names are an internal invariant error.
pub fn render_profile(raw: &str) -> Result<String, FzzError> {
    Profile::parse(raw)
        .map(|profile| profile.render())
        .ok_or_else(|| FzzError::GenericError(format!("unknown template profile: {raw}")))
}

fn minimal_yaml() -> &'static str {
    r#"on:
  change: "**/*"

jobs:
  - name: build
    run: "cargo build"
"#
}

fn parallel_yaml() -> &'static str {
    r#"on:
  change: "src/**"

execution:
  concurrency: 2

jobs:
  - name: lint
    parallel: checks
    run: "cargo clippy"

  - name: test
    parallel: checks
    run: "cargo test"

  - name: package
    run: "cargo build"
"#
}

fn agent_yaml() -> &'static str {
    r#"# Agent loop example: control socket + a verify-style job.
# Next commands:
#   fzz check          # validate this config
#   fzz list           # see the targets
#   fzz run verify     # run once locally
#   fzz watch          # start the watcher + control socket
#   Ctrl-G             # run the complete pipeline once while watching
on:
  change: "**/*"
  socket: .watch.sock

execution:
  concurrency: 2
  recovery_policy: prompt
  recovery_timeout: 60s

jobs:
  - name: verify @agent-final
    run: "cargo test"
    change: "src/**"
    ignore: "target/**"

  - name: lint @quick
    run: "cargo fmt -- --check"
    recovery: "cargo fmt --"
    change: "src/**"
    run_on_init: true

  # MANUAL-TRIGGER-CONTRACT: explicit invocation only (`fzz run` /
  # `fzz ctl run`); never matches filesystem events, never runs at init.
  - name: await-remote
    trigger: manual
    run: "./scripts/await-remote.sh"
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TASK-0097: one typed selector — every accepted name round-trips, and
    /// the name list has no duplicates.
    #[test]
    fn profile_names_round_trip_and_are_unique() {
        for name in Profile::NAMES {
            let profile = Profile::parse(name).unwrap_or_else(|| panic!("{name} must parse"));
            assert_eq!(profile.name(), name);
        }
        let mut names: Vec<&str> = Profile::NAMES.to_vec();
        names.sort_unstable();
        let dedup_count = names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(names.len(), dedup_count, "profile names must be unique");
    }

    #[test]
    fn unknown_profile_names_are_rejected() {
        assert!(Profile::parse("bogus").is_none());
        assert!(Profile::parse("").is_none());
        assert!(Profile::parse("comprehensive ").is_none());
        assert!(render_profile("bogus").is_err());
    }

    /// Every generated artifact parses and validates through the production
    /// parser — no profile can regress below `fzz check` validity.
    #[test]
    fn every_profile_artifact_parses_and_validates() {
        for profile in Profile::NAMES.map(|name| Profile::parse(name).unwrap()) {
            let yaml = profile.render();
            let rules = crate::config::from_yaml(&yaml)
                .unwrap_or_else(|err| panic!("{} must parse: {err:?}", profile.name()));
            crate::rules::validate_rules(&rules)
                .unwrap_or_else(|err| panic!("{} must validate: {err}", profile.name()));
        }
    }

    #[test]
    fn profiles_emit_only_canonical_v2_policy_placements() {
        let parallel = Profile::Parallel.render();
        assert!(parallel.contains("execution:\n  concurrency: 2"));
        assert!(!parallel.contains("on:\n  change: \"src/**\"\n  concurrency:"));

        let agent = Profile::Agent.render();
        assert!(agent.contains("on:\n  change: \"**/*\"\n  socket:"));
        assert!(agent.contains("execution:\n  concurrency: 2"));
        for profile in Profile::NAMES.map(|name| Profile::parse(name).unwrap()) {
            let yaml = profile.render();
            assert!(
                !yaml.contains("version:"),
                "{} must not carry a config version",
                profile.name()
            );
            assert!(
                !yaml.contains("inherits on.output"),
                "{} teaches an old output owner",
                profile.name()
            );
        }
    }

    /// Rendering is deterministic: identical bytes on every call.
    #[test]
    fn rendering_is_deterministic() {
        for name in Profile::NAMES {
            let profile = Profile::parse(name).unwrap();
            assert_eq!(profile.render(), profile.render(), "{name} drifted");
        }
    }
}
