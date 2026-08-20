use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::plan::RunPlan;
use crate::rules::Rules;

/// Why a configured rule was selected or skipped for an explained path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainRule {
    pub name: String,
    /// Change patterns that matched the path.
    pub change_patterns: Vec<String>,
    /// Ignore patterns that matched and win over the change match.
    pub ignore_patterns: Vec<String>,
    /// Effective task cwd for diagnostics.
    pub cwd: String,
    /// Task-local environment names; values remain redacted.
    pub environment_keys: Vec<String>,
    /// Whether this job declares an available recovery; approval is separate.
    pub recovery_available: bool,
    /// Where the effective rule came from (TASK-0023): `task` or `group`
    /// when the responsible pattern was inherited from a group `on` section.
    pub origin: String,
}

/// Result of explaining a path against the configured rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplainResult {
    /// Rules that would run for the path (change matched, no ignore).
    pub matched: Vec<ExplainRule>,
    /// Rules whose change pattern matched but an ignore pattern won.
    pub ignored: Vec<ExplainRule>,
    /// The filtered execution topology (stages + named group occurrences)
    /// after path filtering — the actual run plan preview, same planner as
    /// execution (TASK-0034). Empty when nothing matches.
    pub plan_stages: Vec<PlanStagePreview>,
}

/// One filtered execution stage for `explain` output (TASK-0034): a serial
/// task, or a named parallel group occurrence with its selected members.
/// Barriers and group names are shown without implying completion order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStagePreview {
    Serial { task: String },
    Parallel { group: String, tasks: Vec<String> },
}

/// Execution facts relevant to an explained plan (TASK-0034): the effective
/// scheduler concurrency and the filesystem debounce window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplainFacts {
    pub concurrency: usize,
    pub debounce: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTargetError {
    Missing(String),
    Ambiguous {
        target: String,
        matches: Vec<String>,
    },
}

impl fmt::Display for RunTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunTargetError::Missing(target) => {
                write!(formatter, "No target found for '{}'", target)
            }
            RunTargetError::Ambiguous { target, matches } => write!(
                formatter,
                "Target '{}' is ambiguous; matches: {}",
                target,
                matches.join(", ")
            ),
        }
    }
}

/// # Watches
///
/// Represents all rules in the yaml config loaded.
///
#[derive(Debug, Clone)]
pub struct Watches {
    rules: Vec<Rules>,
    topology: RunPlan,
    root: PathBuf,
    concurrency: usize,
    /// Debounce window per filesystem event batch (TASK-0031). Defaults to
    /// the historical one second unless `on.debounce` configures otherwise.
    debounce: Duration,
    /// Filesystem backend policy (TASK-0037): native, poll, or auto.
    backend: crate::watcher::WatchBackend,
    /// Whether workspace `.gitignore` rules are respected (TASK-0036).
    respect_gitignore: bool,
    /// Effective user-approved recovery policy for failed jobs.
    recovery_policy: crate::config::RecoveryPolicy,
    /// Generation terminal hooks (TASK-0040): success/failure commands.
    hooks: crate::config::GenerationHooks,
    /// Watcher-session terminal hook (TASK-0101): close command. Kept out of
    /// generation executors and consumed only by the shutdown coordinator.
    session_hooks: crate::config::SessionHooks,
    /// The immutable runtime config revision this watch plan was built from
    /// (TASK-0089). Captured before any plan is created; a batch routes
    /// under exactly one revision. None for legacy constructions that never
    /// observe reload.
    revision: Option<crate::config_revision::ConfigRevision>,
    /// Root-anchored gitignore matcher; rebuilt when the gitignore changes.
    /// Interior mutability so routing can refresh before each batch without
    /// an event-loss gap (TASK-0036 §4).
    gitignore: Option<std::sync::Arc<std::sync::Mutex<crate::gitignore::GitignoreMatcher>>>,
}
impl Watches {
    /// Convenience constructor resolving the workspace root from the process
    /// current directory. Keep usage at the outer boundary (composition root
    /// and tests); prefer [`Watches::with_root`] so core behavior does not
    /// depend on hidden process state.
    pub fn new(rules: Vec<Rules>) -> Self {
        let root = std::env::current_dir().expect("Unable to get current directory");
        Watches::with_root(rules, root)
    }

    /// Creates watches anchored at an explicit workspace root.
    pub fn with_root(rules: Vec<Rules>, root: PathBuf) -> Self {
        let concurrency = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        Watches::with_root_and_concurrency(rules, root, concurrency)
    }

    pub fn with_root_and_concurrency(rules: Vec<Rules>, root: PathBuf, concurrency: usize) -> Self {
        assert!(concurrency > 0, "watch concurrency must be positive");
        let topology = RunPlan::from_rules(rules.clone());
        Watches {
            rules,
            topology,
            root,
            concurrency,
            debounce: Duration::from_millis(1000),
            backend: crate::watcher::WatchBackend::Auto,
            respect_gitignore: false,
            gitignore: None,
            recovery_policy: crate::config::RecoveryPolicy::Prompt,
            hooks: crate::config::GenerationHooks::default(),
            session_hooks: crate::config::SessionHooks::default(),
            revision: None,
        }
    }

    /// Overrides the filesystem debounce window (TASK-0031); the default is
    /// the historical one second.
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// The debounce window used per filesystem event batch.
    pub fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Overrides the filesystem backend policy (TASK-0037).
    pub fn with_backend(mut self, backend: crate::watcher::WatchBackend) -> Self {
        self.backend = backend;
        self
    }

    /// The configured filesystem backend policy.
    pub fn backend(&self) -> crate::watcher::WatchBackend {
        self.backend
    }

    /// Enables gitignore respect and builds the root-anchored matcher
    /// (TASK-0036). Explicit config `ignore` rules stay strongest.
    pub fn with_gitignore(mut self, respect: bool) -> Self {
        self.respect_gitignore = respect;
        self.gitignore = respect.then(|| {
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::gitignore::GitignoreMatcher::new(self.root.clone()),
            ))
        });
        self
    }

    /// Whether a root-relative path is excluded by workspace gitignore rules,
    /// refreshing the matcher first when the workspace gitignore changed (no
    /// event-loss gap).
    pub fn gitignored(&self, relative: &std::path::Path) -> bool {
        self.gitignore
            .as_ref()
            .map(|matcher| {
                let mut matcher = matcher.lock().unwrap();
                if matcher.needs_rebuild() {
                    matcher.rebuild();
                }
                matcher.is_ignored(relative)
            })
            .unwrap_or(false)
    }

    /// True when gitignore respect is enabled.
    pub fn respects_gitignore(&self) -> bool {
        self.respect_gitignore
    }

    /// Sets the effective user-approved recovery policy.
    pub fn with_recovery_policy(mut self, policy: crate::config::RecoveryPolicy) -> Self {
        self.recovery_policy = policy;
        self
    }

    /// The effective recovery policy for this frozen watch configuration.
    pub fn recovery_policy(&self) -> crate::config::RecoveryPolicy {
        self.recovery_policy
    }

    /// Overrides run-level terminal hooks (TASK-0040).
    pub fn with_hooks(mut self, hooks: crate::config::GenerationHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// The configured generation terminal hooks.
    pub fn hooks(&self) -> crate::config::GenerationHooks {
        self.hooks.clone()
    }

    /// Overrides the watcher-session close hook (TASK-0101).
    pub fn with_session_hooks(mut self, hooks: crate::config::SessionHooks) -> Self {
        self.session_hooks = hooks;
        self
    }

    /// The configured watcher-session close hook.
    pub fn session_hooks(&self) -> crate::config::SessionHooks {
        self.session_hooks.clone()
    }

    /// Binds the immutable runtime config revision this watch plan was
    /// captured under (TASK-0089, CONFIG-RELOAD-CONTRACT §4). Composition
    /// root sets it once; batches routed through this instance carry it.
    pub fn with_revision(mut self, revision: crate::config_revision::ConfigRevision) -> Self {
        self.revision = Some(revision);
        self
    }

    /// The revision this watch plan is frozen under; None for legacy
    /// constructions that never observe reload.
    pub fn revision(&self) -> Option<&crate::config_revision::ConfigRevision> {
        self.revision.as_ref()
    }

    /// Narrows visible rules while retaining barriers from original topology.
    pub fn select_target(&self, target: &str) -> Option<Self> {
        let rules: Vec<Rules> = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .cloned()
            .collect();
        if rules.is_empty() {
            return None;
        }
        Some(Self {
            rules,
            topology: self
                .topology
                .clone()
                .filter(|rule| rule.name.contains(target)),
            root: self.root.clone(),
            concurrency: self.concurrency,
            debounce: self.debounce,
            backend: self.backend,
            respect_gitignore: self.respect_gitignore,
            gitignore: self.gitignore.clone(),
            recovery_policy: self.recovery_policy,
            hooks: self.hooks.clone(),
            session_hooks: self.session_hooks.clone(),
            revision: self.revision.clone(),
        })
    }

    /// The workspace root this watch planning is anchored to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Replaces the effective watch configuration with a reloaded candidate
    /// (TASK-0090): swaps rules, topology, policy, and revision so later
    /// batches route under the committed revision. Returns the previous
    /// root set so the caller can retire obsolete roots (contract §4).
    pub fn swap_config(&mut self, candidate: Watches) -> Vec<PathBuf> {
        let previous_roots: Vec<PathBuf> = self
            .paths_to_watch()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        *self = candidate;
        previous_roots
    }

    /// Maximum simultaneously active tasks within one parallel group.
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    fn normalize_paths<'a>(&'a self, path: &'a str) -> (PathBuf, Option<String>) {
        let provided_path = Path::new(path);
        let absolute_path = if provided_path.is_absolute() {
            provided_path.to_path_buf()
        } else {
            self.root.join(provided_path)
        };

        let relative_path = match absolute_path.strip_prefix(&self.root) {
            Ok(rel) => Some(format!("/{}", rel.display())),
            Err(_) => None,
        };

        (absolute_path, relative_path)
    }

    /// The root-relative form of a path used for rule matching (TASK-0023):
    /// the deterministic normalized path diagnostics report alongside the raw
    /// event path. Falls back to the provided path when it is outside the
    /// workspace root.
    pub fn normalized_path(&self, path: &str) -> String {
        let (_, relative) = self.normalize_paths(path);
        relative
            .map(|relative| relative.trim_start_matches('/').to_owned())
            .unwrap_or_else(|| path.to_owned())
    }

    /// Returns all configured targets.
    pub fn targets(&self) -> Vec<Rules> {
        self.rules.clone()
    }

    /// Returns rules whose names contain the requested target.
    pub fn target(&self, target: &str) -> Option<Vec<Rules>> {
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .cloned()
            .collect::<Vec<Rules>>();

        if rules.is_empty() {
            return None;
        }

        Some(rules)
    }

    /// Resolves a finite local-run target deterministically.
    ///
    /// An exact task name wins. `@tag` selectors intentionally run every
    /// match. Other substrings must identify one task; multiple matches are
    /// rejected rather than running an accidental superset in CI.
    pub fn run_target_plan(&self, target: &str) -> Result<RunPlan, RunTargetError> {
        let exact_matches = self
            .rules
            .iter()
            .filter(|rule| rule.name == target)
            .collect::<Vec<_>>();
        if exact_matches.len() > 1 {
            return Err(RunTargetError::Ambiguous {
                target: target.to_owned(),
                matches: exact_matches.iter().map(|rule| rule.name.clone()).collect(),
            });
        }
        if exact_matches.len() == 1 {
            return Ok(self.topology.clone().filter(|rule| rule.name == target));
        }

        let matches = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(RunTargetError::Missing(target.to_owned()));
        }
        if matches.len() > 1 && !target.starts_with('@') {
            return Err(RunTargetError::Ambiguous {
                target: target.to_owned(),
                matches: matches.iter().map(|rule| rule.name.clone()).collect(),
            });
        }

        Ok(self
            .topology
            .clone()
            .filter(|rule| rule.name.contains(target)))
    }

    /// Selects a target without collapsing barriers around unmatched rules.
    pub fn target_plan(&self, target: &str) -> Option<RunPlan> {
        let plan = self
            .topology
            .clone()
            .filter(|rule| rule.name.contains(target));
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Selects matching tasks while retaining original stage occurrences.
    pub fn watch_plan(&self, path: &str) -> Option<RunPlan> {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();
        let plan = self.topology.clone().filter(|rule| {
            let ignored_by_absolute = rule.ignore_absolute(absolute_path_str);
            let ignored_by_relative = relative_path
                .as_ref()
                .map(|relative| rule.ignore_relative(relative))
                .unwrap_or(false);
            if ignored_by_absolute || ignored_by_relative {
                return false;
            }

            // TASK-0036: gitignore applies only when enabled and never beats
            // the explicit config `ignore` (checked above). Root-relative
            // matching keeps behavior identical for absolute inputs.
            if self.respect_gitignore {
                if let Some(relative) = relative_path.as_deref() {
                    // The normalized relative form has a leading '/' that the
                    // ignore crate rejects; strip it for root-relative input.
                    let relative = relative.trim_start_matches('/');
                    if self.gitignored(Path::new(relative)) {
                        return false;
                    }
                }
            }

            let watched_by_absolute = rule.watch_absolute(absolute_path_str);
            let watched_by_relative = relative_path
                .as_ref()
                .map(|relative| rule.watch_relative(relative))
                .unwrap_or(false);
            watched_by_absolute || watched_by_relative
        });
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Routes one normalized event batch to zero or one generation (contract
    /// §1): scans the changed paths in deterministic order and returns the
    /// plan of the FIRST path that matches (change matched, not ignored),
    /// plus that trigger path for template expansion. A batch whose paths are
    /// all unmatched or ignored yields None.
    pub fn watch_plan_batch(&self, paths: &[String]) -> Option<(RunPlan, String)> {
        let mut sorted = paths.to_vec();
        sorted.sort();
        for path in sorted {
            if let Some(plan) = self.watch_plan(&path) {
                return Some((plan, path));
            }
        }
        None
    }

    /// Returns the commands for first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<Rules>> {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();

        let cmds = self
            .rules
            .iter()
            .filter(|&r| {
                let ignored_by_absolute = r.ignore_absolute(absolute_path_str);
                let ignored_by_relative = relative_path
                    .as_ref()
                    .map(|rel| r.ignore_relative(rel))
                    .unwrap_or(false);

                if ignored_by_absolute || ignored_by_relative {
                    return false;
                }

                let watched_by_absolute = r.watch_absolute(absolute_path_str);
                let watched_by_relative = relative_path
                    .as_ref()
                    .map(|rel| r.watch_relative(rel))
                    .unwrap_or(false);

                watched_by_absolute || watched_by_relative
            })
            .cloned()
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Selects init tasks while retaining original stage occurrences.
    pub fn run_on_init_plan(&self) -> Option<RunPlan> {
        let plan = self.topology.clone().filter(Rules::run_on_init);
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Returns the commands for the rules that should run on init
    ///
    pub fn run_on_init(&self) -> Option<Vec<Rules>> {
        let cmds = self
            .rules
            .iter()
            .filter(|&r| r.run_on_init())
            .cloned()
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Explains which configured rules match, ignore, or miss `path`.
    ///
    /// Reuses the exact matching policy of [`Watches::watch`] (same path
    /// normalization and rule matchers); this method only reports *which*
    /// change and ignore patterns matched per rule. It never starts a watcher
    /// or executes a task.
    pub fn explain(&self, path: &str) -> ExplainResult {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();

        let mut matched = vec![];
        let mut ignored = vec![];

        for rule in &self.rules {
            // Mirror Watches::watch exactly: absolute patterns match the
            // absolute path; relative patterns match the root-relative path.
            let mut change_patterns = rule.watch_absolute_patterns(absolute_path_str);
            if let Some(rel) = relative_path.as_ref() {
                change_patterns.extend(rule.watch_relative_patterns(rel));
            }
            if change_patterns.is_empty() {
                continue;
            }

            let mut ignore_patterns = rule.ignore_absolute_patterns(absolute_path_str);
            if let Some(rel) = relative_path.as_ref() {
                ignore_patterns.extend(rule.ignore_relative_patterns(rel));
            }
            let cwd = rule
                .cwd()
                .map(|cwd| self.root.join(cwd))
                .unwrap_or_else(|| self.root.clone())
                .display()
                .to_string();
            let environment_keys = rule.environment().keys().cloned().collect();
            // TASK-0036: gitignore applies only when enabled and a change
            // pattern matched; the explicit config `ignore` (above) stays
            // strongest. The source label tells the user exactly where the
            // exclusion came from.
            let gitignored = ignore_patterns.is_empty()
                && self.respect_gitignore
                && relative_path
                    .as_deref()
                    .map(|rel| self.gitignored(Path::new(rel.trim_start_matches('/'))))
                    .unwrap_or(false);
            if ignore_patterns.is_empty() && !gitignored {
                let origin = Self::origin_for(rule, change_patterns.first(), None);
                matched.push(ExplainRule {
                    name: rule.name.clone(),
                    change_patterns,
                    ignore_patterns: vec![],
                    cwd,
                    environment_keys,
                    recovery_available: rule.recovery_commands().is_some(),
                    origin,
                });
            } else {
                let mut effective_ignores = ignore_patterns;
                if gitignored {
                    effective_ignores.push(".gitignore".to_owned());
                }
                let origin = Self::origin_for(rule, None, effective_ignores.first());
                ignored.push(ExplainRule {
                    name: rule.name.clone(),
                    change_patterns,
                    ignore_patterns: effective_ignores,
                    cwd,
                    environment_keys,
                    recovery_available: rule.recovery_commands().is_some(),
                    origin,
                });
            }
        }

        ExplainResult {
            matched,
            ignored,
            // TASK-0034: the filtered execution topology uses the exact same
            // planner as execution (watch_plan), so the preview can never
            // drift from what would actually run.
            plan_stages: self
                .watch_plan(path)
                .map(|plan| {
                    plan.stages
                        .into_iter()
                        .map(|stage| match stage {
                            crate::plan::Stage::Serial(task) => {
                                PlanStagePreview::Serial { task: task.name }
                            }
                            crate::plan::Stage::Parallel { group, tasks } => {
                                // The stage carries the bare group name; the
                                // occurrence identity lives on the member
                                // tasks (e.g. `checks#2`), so previews show
                                // the same occurrence label as execution.
                                let occurrence = tasks
                                    .first()
                                    .and_then(|t| t.group_occurrence.clone())
                                    .unwrap_or_else(|| group.clone());
                                PlanStagePreview::Parallel {
                                    group: occurrence,
                                    tasks: tasks.into_iter().map(|t| t.name).collect(),
                                }
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The effective-rule origin for one rule decision: `group` when the
    /// responsible pattern was inherited from a group `on` section, else
    /// `task`. The responsible pattern is the first matching change pattern
    /// (or the first matching ignore pattern when the ignore won).
    fn origin_for(rule: &Rules, change: Option<&String>, ignore: Option<&String>) -> String {
        let inherited = rule.inherited_patterns();
        let responsible = change.or(ignore);
        match responsible {
            Some(pattern) if inherited.iter().any(|p| p == pattern) => "group".to_owned(),
            _ => "task".to_owned(),
        }
    }

    /// Deterministic minimal subscription-root plan (TASK-0085 §3, TASK-0086):
    /// for every change pattern, the literal directory prefix; a partly or
    /// fully missing prefix resolves to its nearest existing ancestor. The
    /// set is canonicalized, deduplicated, containment-minimized (a root
    /// inside an already-watched root is dropped), workspace-bounded for
    /// relative patterns, and stable (sorted). The workspace root itself is
    /// watched only when no narrower safe ancestor exists.
    pub fn subscription_roots(&self) -> Vec<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for rule in &self.rules {
            for pattern in rule.watch_patterns() {
                let candidate = self.root_for_pattern(&pattern);
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        // Containment minimization: a candidate inside another candidate is
        // already covered by the ancestor's recursive watch.
        let mut minimal: Vec<PathBuf> = candidates
            .iter()
            .filter(|candidate| {
                !candidates
                    .iter()
                    .any(|other| other != *candidate && candidate.starts_with(other))
            })
            .cloned()
            .collect();
        minimal.sort();
        minimal.dedup();
        minimal
    }

    /// Existing paths that need an initial modification baseline.
    ///
    /// Unlike backend subscription roots, baseline paths preserve exact files
    /// and never fall back from a missing literal prefix to an existing
    /// ancestor. This keeps startup work inside configured pattern scope: an
    /// exact `Cargo.toml` pattern baselines that file without walking every
    /// build artifact under the workspace root.
    pub(crate) fn baseline_paths(&self) -> Vec<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for rule in &self.rules {
            for pattern in rule.watch_patterns() {
                let absolute = if pattern.starts_with('/') {
                    PathBuf::from(pattern)
                } else {
                    self.root.join(pattern)
                };
                let prefix = Self::literal_prefix(&absolute);
                if prefix.exists() && !candidates.contains(&prefix) {
                    candidates.push(prefix);
                }
            }
        }

        let mut minimal: Vec<PathBuf> = candidates
            .iter()
            .filter(|candidate| {
                !candidates
                    .iter()
                    .any(|other| other != *candidate && candidate.starts_with(other))
            })
            .cloned()
            .collect();
        minimal.sort();
        minimal.dedup();
        minimal
    }

    /// One pattern's subscription root: its literal prefix (segments until a
    /// glob metacharacter), resolved to the nearest existing ancestor. An
    /// exact-file pattern watches its parent directory. A missing absolute
    /// prefix never produces the filesystem root — the literal prefix is
    /// returned so the backend warns actionably instead of silently missing.
    fn root_for_pattern(&self, pattern: &str) -> PathBuf {
        let absolute = if pattern.starts_with('/') {
            PathBuf::from(pattern)
        } else {
            self.root.join(pattern)
        };
        let prefix = Self::literal_prefix(&absolute);
        Self::nearest_existing_ancestor_or_self(&prefix)
    }

    /// The literal path prefix: components until the first glob
    /// metacharacter (`* ? [ {`), preserving absolute-ness.
    fn literal_prefix(path: &Path) -> PathBuf {
        use std::path::Component;
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) => out.push("/"),
                Component::RootDir => out.push("/"),
                Component::CurDir => {}
                Component::ParentDir => out.push(".."),
                Component::Normal(seg) => {
                    let seg = seg.to_string_lossy();
                    if seg.contains('*')
                        || seg.contains('?')
                        || seg.contains('[')
                        || seg.contains('{')
                    {
                        break;
                    }
                    out.push(seg.as_ref());
                }
            }
        }
        out
    }

    /// The path itself when it exists as a directory, else its parent when
    /// it exists as a file, else the nearest existing ancestor. Walking up
    /// never returns the filesystem root: an unwatchable prefix returns
    /// itself so the backend emits an actionable warning.
    fn nearest_existing_ancestor_or_self(path: &Path) -> PathBuf {
        if path.exists() {
            if path.is_dir() {
                return path.to_path_buf();
            }
            return path
                .parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| path.to_path_buf());
        }
        let mut current = path.to_path_buf();
        while let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            current = parent.to_path_buf();
            if current == Path::new("/") {
                // Never watch the whole filesystem; the backend will warn on
                // the literal prefix instead (contract §8).
                return path.to_path_buf();
            }
            if current.exists() {
                return current;
            }
        }
        path.to_path_buf()
    }

    /// Returns the minimal subscription roots as display strings, for the
    /// backend adapter and startup diagnostics (contract §8).
    pub fn paths_to_watch(&self) -> Option<Vec<String>> {
        let roots = self.subscription_roots();
        if roots.is_empty() {
            return None;
        }
        Some(
            roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
        )
    }

    /// Subscription roots that will observe `path` (contract §8): the roots
    /// whose recursive watch covers the path. Used by `explain` to name
    /// coverage for future paths explicitly.
    pub fn covering_roots(&self, path: &str) -> Vec<String> {
        let (absolute_path, _) = self.normalize_paths(path);
        self.subscription_roots()
            .into_iter()
            .filter(|root| absolute_path.starts_with(root))
            .map(|root| root.display().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    extern crate glob;
    extern crate yaml_rust2;

    use super::*;
    use crate::config;
    use crate::plan::Stage;
    use crate::rules;
    use std::env;

    fn get_absolute_path(path: &str) -> String {
        let mut absolute_path = env::current_dir().unwrap();
        absolute_path.push(path);
        absolute_path.to_str().unwrap().to_string()
    }

    #[test]
    fn it_loads_from_args() {
        let watches = Watches::new(
            config::from_argv(vec![".".to_owned()], vec!["cargo build".to_owned()])
                .expect("Error while parsing rules from string"),
        );

        assert!(watches.watch(&get_absolute_path("src/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path("test/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path(".")).is_some());

        let result = rules::commands(watches.watch(&get_absolute_path(".")).unwrap());
        assert_eq!(vec!["cargo build"], result);
    }

    #[test]
    fn it_watches_test_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        assert!(watches.watch(&get_absolute_path("tests/test.rs")).is_some());
        assert!(watches.watch("tests/tests.rs").is_some());
        assert!(watches.watch("tests/ruby.rb").is_some());
        assert!(watches.watch("tests/folder/other.rs").is_some())
    }

    #[test]
    fn it_watches_specific_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: './tests/foo/bar.rs'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        assert!(watches.watch("./tests/foo/bar.rs").is_some())
    }

    #[test]
    fn it_anchors_relative_patterns_to_root() {
        let file_content = "
        - name: txt files
          run: 'echo txt'
          change: 'src/*.txt'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let root = std::env::current_dir().unwrap();
        let inside = root.join("src/foo.txt");
        let outside = root.join(".tmp/src/foo.txt");

        assert!(watches.watch(inside.to_str().unwrap()).is_some());
        assert!(watches.watch(outside.to_str().unwrap()).is_none());
    }

    #[test]
    fn it_explains_matched_ignored_and_unmatched_paths() {
        let file_content = "
        - name: my tests
          cwd: crates/tests
          env: { TOKEN: secret }
          run: 'cargo tests'
          change: 'tests/**'
          ignore: 'tests/ignored/**'

        - name: docs
          run: 'echo docs'
          change: 'docs/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let matched = watches.explain(&get_absolute_path("tests/foo.rs"));
        assert_eq!(matched.matched.len(), 1, "one rule matches tests/**");
        assert_eq!(matched.matched[0].name, "my tests");
        assert_eq!(matched.matched[0].change_patterns, vec!["tests/**"]);
        assert!(matched.matched[0].ignore_patterns.is_empty());
        assert!(matched.matched[0].cwd.ends_with("crates/tests"));
        assert_eq!(matched.matched[0].environment_keys, vec!["TOKEN"]);
        assert_eq!(matched.matched[0].origin, "task");
        assert!(!format!("{:?}", matched.matched[0]).contains("secret"));
        assert!(matched.ignored.is_empty());

        let ignored = watches.explain(&get_absolute_path("tests/ignored/foo.rs"));
        assert!(ignored.matched.is_empty());
        assert_eq!(ignored.ignored.len(), 1, "ignore wins over change match");
        assert_eq!(ignored.ignored[0].name, "my tests");
        assert_eq!(ignored.ignored[0].ignore_patterns, vec!["tests/ignored/**"]);
        assert_eq!(ignored.ignored[0].change_patterns, vec!["tests/**"]);
        assert_eq!(ignored.ignored[0].origin, "task");

        let unmatched = watches.explain(&get_absolute_path("unknown/path.rs"));
        assert!(unmatched.matched.is_empty());
        assert!(unmatched.ignored.is_empty());
    }

    #[test]
    fn it_explains_relative_paths_the_same_as_absolute() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let absolute = watches.explain(&get_absolute_path("tests/foo.rs"));
        let relative = watches.explain("tests/foo.rs");

        assert_eq!(absolute.matched.len(), 1);
        assert_eq!(
            relative.matched[0].change_patterns,
            absolute.matched[0].change_patterns
        );
    }

    #[test]
    fn it_explains_absolute_pattern_paths() {
        let file_content = "
        - name: tmp watcher
          run: 'echo tmp'
          change: '/tmp/funzzy-explain-*/*.txt'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let matched = watches.explain("/tmp/funzzy-explain-1/foo.txt");
        assert_eq!(matched.matched.len(), 1);
        assert_eq!(matched.matched[0].name, "tmp watcher");
        assert_eq!(
            matched.matched[0].change_patterns,
            vec!["/tmp/funzzy-explain-*/*.txt"]
        );
    }

    #[test]
    fn it_explains_merged_group_rules_from_nested_groups() {
        let content = std::fs::read_to_string("examples/nested-groups.yml").expect("read example");
        let watches = Watches::new(config::from_yaml(&content).expect("Error parsing yaml"));

        // frontend-build inherits the group change patterns; the effective
        // rule origin must be reported as the group, not the task.
        let matched = watches.explain("src/frontend/App.tsx");
        let frontend = matched
            .matched
            .iter()
            .find(|rule| rule.name == "frontend-build")
            .expect("frontend-build must match");
        assert_eq!(
            frontend.origin, "group",
            "group-inherited patterns must be reported with group origin"
        );
        let names: Vec<&str> = matched
            .matched
            .iter()
            .map(|rule| rule.name.as_str())
            .collect();
        assert!(
            names.contains(&"frontend-build"),
            "group-merged rule must match: {:?}",
            names
        );

        // Group ignore wins over the inherited change match.
        let ignored = watches.explain("src/frontend/server.log");
        let ignored_names: Vec<&str> = ignored
            .ignored
            .iter()
            .map(|rule| rule.name.as_str())
            .collect();
        assert!(
            ignored_names.contains(&"frontend-build"),
            "group ignore must win: {:?}",
            ignored_names
        );
    }

    #[test]
    fn it_doesnot_watch_test_path() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        assert!(watches.watch(&get_absolute_path("events.yaml")).is_none());
        assert!(watches.watch("tests/").is_none());
        assert!(watches.watch("tests/test.rs").is_none());
        assert!(watches.watch("tests/folder/other.rs").is_none());
    }

    #[test]
    fn it_creates_a_list_of_shell_commands() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result[0])
    }

    #[test]
    fn it_works_with_multiples_items() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let result = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!("cargo test", result[0]);

        let result_src = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result_src[0]);
    }

    #[test]
    fn it_allows_many_rules_watching_same_path() {
        let file_content = "
        - name: same path
          run: 'echo same'
          change: '**'

        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));

        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo build"], result);

        let result_multiple = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo test"], result_multiple);
    }

    #[test]
    fn it_ignores_pattern() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: 'src/test/**'
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_a_list_of_patterns() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/tmp/test.txt").is_none());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_nested_tooling_cache_dirs_and_the_directory_event() {
        // Tooling may emit an event for either the cache directory or a file in it.
        // Both patterns are needed because `**/.pi/**` does not match `tests/.pi`.
        let file_content = "
            - name: lint
              run: 'cargo fmt'
              change: 'tests/**'
              ignore: ['**/.pi', '**/.pi/**']
        ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        assert!(watches.watch("tests/foo.rs").is_some());
        assert!(watches.watch("tests/.pi").is_none());
        assert!(watches.watch("tests/.pi/ast-index.sqlite").is_none());
    }

    #[test]
    fn it_ignores_tooling_cache_in_real_watch_config() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config =
            std::fs::read_to_string(here.join(".watch.yaml")).expect("missing .watch.yaml");
        let watches = Watches::new(config::from_yaml(&config).expect("Error parsing yaml"));

        for path in ["tests/.pi", "tests/.pi/ast-index.sqlite"] {
            let matched: Vec<String> = watches
                .watch(path)
                .map(|rules| rules.into_iter().map(|r| r.name).collect())
                .unwrap_or_default();
            assert!(
                matched.is_empty(),
                "tooling cache path '{}' must not trigger any task, got: {:?}",
                path,
                matched
            );
        }
    }

    #[test]
    fn it_returns_on_init_rules() {
        let file_content = "
            - name: my source
              run: 'cargo build'
              change: 'src/**'
              run_on_init: true

            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'
              run_on_init: true

            - name: other
              run: 'cargo test'
              change: 'test/**'
            ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        let results = rules::commands(watches.run_on_init().unwrap());

        assert_eq!(
            results,
            vec![
                "cargo build".to_string(),
                "cat foo".to_string(),
                "cat bar".to_string(),
            ]
        );
    }

    #[test]
    fn it_returns_rules_with_absolute_path_and_current_dir() {
        let file_content = "
            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'

            - name: rule with absolute path
              run: 'cargo build'
              change: 
                - 'src/**'
                - '/tmp/**'
                - '/User/**'

            - name: it does not consider the ignored rules
              run: 'cargo test'
              change: 'test/**'
              ignored: '/test/**'

            - name: another with absolute path
              run: echo 'absolute paths'
              change: 
                - '/dev/**'
                - '/usr/**'
                - '/etc/**'
            ";
        let watches = Watches::new(config::from_yaml(file_content).expect("Error parsing yaml"));
        let results = watches.paths_to_watch().expect("No rules found");

        let current_dir = std::env::current_dir().expect("Unable to get current directory");
        // Minimal nearest-existing-ancestor roots: `src/**` exists → `src`;
        // `test/**` has no existing `test/` dir → resolves to the workspace
        // root, which then contains `src` (containment-minimized away);
        // absolute dirs that exist are watched directly; `/User` does not
        // exist and never resolves to `/` — its literal prefix is kept so the
        // backend warns actionably (contract §8).
        let mut expected = vec![
            current_dir.display().to_string(),
            "/User".to_owned(),
            "/dev".to_owned(),
            "/etc".to_owned(),
            "/tmp".to_owned(),
            "/usr".to_owned(),
        ];
        expected.sort();
        assert_eq!(results, expected);
    }

    #[test]
    fn it_uses_injected_root_with_spaces_for_relative_patterns() {
        let file_content = "
        - name: txt files
          run: 'echo txt'
          change: 'src/*.txt'
    ";
        let rules = config::from_yaml(file_content).expect("Error parsing yaml");
        let root =
            std::env::temp_dir().join(format!("funzzy root with spaces {}", std::process::id()));
        let watches = Watches::with_root(rules, root.clone());

        let inside = root.join("src/foo.txt");
        let outside = std::env::temp_dir()
            .join(format!("funzzy elsewhere {}", std::process::id()))
            .join("src/foo.txt");

        assert!(
            watches.watch(inside.to_str().unwrap()).is_some(),
            "relative patterns must anchor to the injected root"
        );
        assert!(
            watches.watch(outside.to_str().unwrap()).is_none(),
            "paths outside the injected root must not match relative patterns"
        );
    }

    #[test]
    fn it_matches_absolute_patterns_outside_the_injected_root() {
        let file_content = "
        - name: outside
          run: 'echo outside'
          change: '/tmp/**'
    ";
        let rules = config::from_yaml(file_content).expect("Error parsing yaml");
        let root = std::env::temp_dir().join(format!("funzzy-root-{}", std::process::id()));
        let watches = Watches::with_root(rules, root);

        let outside = std::path::Path::new("/tmp/funzzy-outside-marker/foo.txt");
        assert!(
            watches.watch(outside.to_str().unwrap()).is_some(),
            "absolute patterns must still match outside the injected root"
        );
    }

    #[test]
    fn local_run_target_prefers_exact_allows_tags_and_rejects_ambiguity() {
        let rules = ["build", "build docs", "lint @quick", "test @quick"]
            .iter()
            .map(|name| {
                Rules::new(
                    (*name).to_owned(),
                    vec!["true".to_owned()],
                    vec!["src/**".to_owned()],
                    vec![],
                    false,
                )
            })
            .collect();
        let watches = Watches::new(rules);

        assert_eq!(
            watches
                .run_target_plan("build")
                .expect("exact target")
                .task_names(),
            vec!["build"]
        );
        assert_eq!(
            watches
                .run_target_plan("@quick")
                .expect("tag target")
                .task_names(),
            vec!["lint @quick", "test @quick"]
        );
        assert!(matches!(
            watches.run_target_plan("quick"),
            Err(RunTargetError::Ambiguous { .. })
        ));
        assert_eq!(
            watches.run_target_plan("missing"),
            Err(RunTargetError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn target_selection_keeps_separated_group_occurrences_and_concurrency() {
        let rules = vec![
            Rules::new(
                "A selected".to_owned(),
                vec!["echo a".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned()),
            Rules::new(
                "separator".to_owned(),
                vec!["echo separator".to_owned()],
                vec!["other/**".to_owned()],
                vec![],
                false,
            ),
            Rules::new(
                "C selected".to_owned(),
                vec!["echo c".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned()),
        ];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 2)
            .select_target("selected")
            .expect("selected targets");
        let plan = watches.target_plan("selected").expect("target plan");

        assert_eq!(watches.concurrency(), 2);
        assert_eq!(plan.stages.len(), 2);
        assert!(plan
            .stages
            .iter()
            .all(|stage| matches!(stage, Stage::Parallel { tasks, .. } if tasks.len() == 1)));
    }

    #[test]
    fn subscription_roots_existing_prefix_is_watched_directly() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-roots-{}-{}",
            std::process::id(),
            "existing"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("src")).unwrap();
        std::fs::create_dir_all(scratch.join("tests")).unwrap();

        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned(), "tests/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert_eq!(
            roots,
            vec![scratch.join("src"), scratch.join("tests")],
            "existing literal prefixes are the roots; no root fallback"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_missing_prefix_watches_nearest_existing_ancestor() {
        let scratch =
            std::env::temp_dir().join(format!("funzzy-roots-{}-{}", std::process::id(), "missing"));
        let _ = std::fs::remove_dir_all(&scratch);
        // future/ exists but future/deep/src does not.
        std::fs::create_dir_all(scratch.join("future")).unwrap();

        let rules = vec![Rules::new(
            "future build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["future/deep/src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert_eq!(
            roots,
            vec![scratch.join("future")],
            "missing nested prefix resolves to the nearest existing ancestor"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_entirely_missing_relative_prefix_falls_back_to_workspace_root() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-roots-{}-{}",
            std::process::id(),
            "all-missing"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let rules = vec![Rules::new(
            "future build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["future/deep/src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert_eq!(
            roots,
            vec![scratch.clone()],
            "no existing ancestor below the workspace root falls back to the root itself"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_absolute_missing_prefix_never_watches_filesystem_root() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-roots-{}-{}",
            std::process::id(),
            "abs-missing"
        ));
        std::fs::create_dir_all(&scratch).unwrap();

        let rules = vec![Rules::new(
            "outside".to_owned(),
            vec!["echo outside".to_owned()],
            vec!["/definitely-not-a-real-funzzy-dir-12345/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert!(
            !roots.iter().any(|root| root == Path::new("/")),
            "an unwatchable absolute prefix must never produce the filesystem root: {:?}",
            roots
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_dedupe_and_containment_minimize() {
        let scratch =
            std::env::temp_dir().join(format!("funzzy-roots-{}-{}", std::process::id(), "overlap"));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("src/deep")).unwrap();
        std::fs::create_dir_all(scratch.join("src/other")).unwrap();

        let rules = vec![Rules::new(
            "overlap".to_owned(),
            vec!["echo x".to_owned()],
            vec![
                "src/**".to_owned(),
                "src/deep/**".to_owned(),
                "src/other/*.rs".to_owned(),
                "src/**".to_owned(),
            ],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert_eq!(
            roots,
            vec![scratch.join("src")],
            "contained roots are dropped; duplicates collapse; stable order"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_exact_file_pattern_watches_its_parent() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-roots-{}-{}",
            std::process::id(),
            "exact-file"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("crates/tests")).unwrap();
        std::fs::write(scratch.join("crates/tests/foo.rs"), "x").unwrap();

        let rules = vec![Rules::new(
            "exact".to_owned(),
            vec!["echo x".to_owned()],
            vec!["crates/tests/foo.rs".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        assert_eq!(
            roots,
            vec![scratch.join("crates/tests")],
            "an exact file pattern watches its parent directory"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn baseline_paths_keep_exact_files_without_scanning_their_parent() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-baseline-{}-{}",
            std::process::id(),
            "exact-file"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("src")).unwrap();
        std::fs::create_dir_all(scratch.join("target/debug/deps")).unwrap();
        std::fs::write(scratch.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(scratch.join("target/debug/deps/stale.rcgu.o"), "object").unwrap();

        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["Cargo.toml".to_owned(), "src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        assert_eq!(
            watches.baseline_paths(),
            vec![scratch.join("Cargo.toml"), scratch.join("src")],
            "baseline scope follows pattern prefixes instead of broad backend roots"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn baseline_paths_skip_missing_literal_prefixes() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-baseline-{}-{}",
            std::process::id(),
            "missing-prefix"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let rules = vec![Rules::new(
            "future".to_owned(),
            vec!["echo future".to_owned()],
            vec!["future/deep/src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        assert!(
            watches.baseline_paths().is_empty(),
            "missing future prefixes have no pre-existing files to baseline"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn subscription_roots_stable_across_hash_order_and_sorted() {
        let scratch =
            std::env::temp_dir().join(format!("funzzy-roots-{}-{}", std::process::id(), "stable"));
        let _ = std::fs::remove_dir_all(&scratch);
        for dir in ["zeta", "alpha", "mid", "beta"] {
            std::fs::create_dir_all(scratch.join(dir)).unwrap();
        }

        let rules = vec![Rules::new(
            "stable".to_owned(),
            vec!["echo x".to_owned()],
            vec!["zeta/**".to_owned(), "alpha/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        let roots = watches.subscription_roots();
        let expected = vec![scratch.join("alpha"), scratch.join("zeta")];
        assert_eq!(roots, expected, "roots are sorted deterministically");
        assert_eq!(
            watches.subscription_roots(),
            expected,
            "stable across calls"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn covering_roots_names_the_root_that_will_observe_a_future_path() {
        let scratch = std::env::temp_dir().join(format!(
            "funzzy-roots-{}-{}",
            std::process::id(),
            "covering"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("future")).unwrap();

        let rules = vec![Rules::new(
            "future build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["future/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root(rules, scratch.clone());

        // The path does not exist yet; explain must still name the root that
        // will observe it once created.
        let covering = watches.covering_roots("future/deep/nested/out.txt");
        assert_eq!(covering, vec![scratch.join("future").display().to_string()]);

        let outside = watches.covering_roots("other/never.txt");
        assert!(
            outside.is_empty(),
            "paths outside any root are not covered: {outside:?}"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn one_batch_maps_to_one_generation_via_first_deterministic_match() {
        let rules = vec![
            Rules::new(
                "build".to_owned(),
                vec!["echo build".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                false,
            ),
            Rules::new(
                "docs".to_owned(),
                vec!["echo docs".to_owned()],
                vec!["docs/**".to_owned()],
                vec![],
                false,
            ),
        ];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        // Deterministic order: docs/** sorts before src/**, so "docs/a.md" is
        // the trigger even though "src/main.rs" also matches.
        let (plan, trigger) = watches
            .watch_plan_batch(&["src/main.rs".to_owned(), "docs/a.md".to_owned()])
            .expect("batch matches");
        assert_eq!(trigger, "docs/a.md");
        assert_eq!(plan.task_names(), vec!["docs".to_owned()]);
    }

    #[test]
    fn batch_with_only_ignored_paths_yields_no_generation() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec!["src/generated/**".to_owned()],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        assert!(
            watches
                .watch_plan_batch(&["src/generated/out.rs".to_owned()])
                .is_none(),
            "ignored-only batch must not schedule"
        );
    }

    #[test]
    fn batch_with_ignored_and_matching_paths_uses_the_matching_one() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec!["src/generated/**".to_owned()],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        let (plan, trigger) = watches
            .watch_plan_batch(&["src/generated/out.rs".to_owned(), "src/main.rs".to_owned()])
            .expect("matching path must win");
        assert_eq!(trigger, "src/main.rs");
        assert_eq!(plan.task_names(), vec!["build".to_owned()]);
    }

    #[test]
    fn empty_batch_never_schedules() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);
        assert!(watches.watch_plan_batch(&[]).is_none());
    }

    #[test]
    fn it_returns_an_error_when_fail_to_load_config_file() {
        // assert!(config::from_yaml(
        //     &r#"
        // - name: run tests
        //   run: [
        //     "yarn test {{filepath}}",
        //     "echo '{{filepath}}' | sed -r 's\/.tsx/\/'"
        //   ]
        //   change: 'src/**'
        // "#
        // )
        // .is_err());

        assert!(config::from_yaml(
            r#"
        - name: run tests
          run: [
            "yarn test {{filepath}}", 
          change: 'src/**'
        "#
        )
        .is_err());

        assert!(config::from_yaml(
            r#"
        - name: other
          run: 'cargo test'
          change: 'test/**'
        "#
        )
        .is_ok());
    }
}

#[cfg(test)]
mod explain_plan_tests {
    use super::*;

    fn config(content: &str) -> Vec<Rules> {
        crate::config::from_yaml(content).expect("parse config")
    }

    #[test]
    fn explain_plan_shows_serial_stages_in_order() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    run: echo a\n    change: 'src/**'\n  - name: b\n    run: echo b\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(result.matched.len(), 2);
        assert_eq!(
            result.plan_stages,
            vec![
                PlanStagePreview::Serial {
                    task: "a".to_owned()
                },
                PlanStagePreview::Serial {
                    task: "b".to_owned()
                },
            ]
        );
    }

    #[test]
    fn explain_plan_shows_parallel_group_occurrence() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    parallel: checks\n    run: echo a\n    change: 'src/**'\n  - name: b\n    parallel: checks\n    run: echo b\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(
            result.plan_stages,
            vec![PlanStagePreview::Parallel {
                group: "checks#1".to_owned(),
                tasks: vec!["a".to_owned(), "b".to_owned()],
            }]
        );
    }

    #[test]
    fn explain_plan_keeps_separated_group_occurrences() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    parallel: x\n    run: echo a\n    change: 'src/**'\n  - name: sep\n    run: echo sep\n    change: 'src/**'\n  - name: c\n    parallel: x\n    run: echo c\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(
            result.plan_stages,
            vec![
                PlanStagePreview::Parallel {
                    group: "x#1".to_owned(),
                    tasks: vec!["a".to_owned()],
                },
                PlanStagePreview::Serial {
                    task: "sep".to_owned()
                },
                PlanStagePreview::Parallel {
                    group: "x#2".to_owned(),
                    tasks: vec!["c".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn explain_plan_excludes_ignored_and_unmatched_tasks() {
        let watches = Watches::new(config(
            "jobs:\n  - name: hit\n    run: echo hit\n    change: 'src/**'\n  - name: ignored\n    run: echo ignored\n    change: 'src/**'\n    ignore: 'src/**'\n  - name: other\n    run: echo other\n    change: 'docs/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.ignored.len(), 1);
        assert_eq!(
            result.plan_stages,
            vec![PlanStagePreview::Serial {
                task: "hit".to_owned()
            }]
        );
    }

    #[test]
    fn explain_plan_is_empty_for_unmatched_path() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    run: echo a\n    change: 'src/**'\n",
        ));
        let result = watches.explain("docs/x.md");
        assert!(result.matched.is_empty());
        assert!(result.plan_stages.is_empty());
    }
}
