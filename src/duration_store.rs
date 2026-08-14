//! Bounded XDG state persistence for run duration history (TASK-0053).
//!
//! Adapter responsibilities (contract `docs/RUN-DURATION-ESTIMATES-CONTRACT.md`
//! §5): resolve the state path under `${XDG_STATE_HOME:-~/.local/state}`,
//! strict versioned decode, bounded profile/sample counts, atomic temp-file
//! replacement, `0600` permissions, corrupt-file quarantine with one warning
//! and empty recovery, and an explicit single-writer policy. The pure
//! estimator (`duration_history`) stays independent of serde and filesystem.

use crate::duration_history::{DurationHistory, ProfileSnapshot};
use crate::plan::hex;
use serde_derive::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// State schema version; unknown versions are quarantined (contract §5).
pub const STATE_SCHEMA_VERSION: u64 = 1;
/// File name carries the version so a future migration keeps old files.
pub const STATE_FILE_NAME: &str = "run-durations-v1.json";
/// Absolute cap on profile count in one file (bounded allocation).
pub const MAX_PROFILES: usize = 10_000;
/// Absolute cap on file size in bytes; oversized files are rejected.
pub const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;
/// Subdirectory under the state root that holds per-workspace stores.
const WORKSPACES_DIR: &str = "workspaces";

/// Default state root per XDG: `${XDG_STATE_HOME:-~/.local/state}`.
/// Pure; tests inject `XDG_STATE_HOME`/`HOME` via env overrides.
pub fn default_state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local").join("state")
}

/// Stable workspace identity: SHA-256 over the canonical workspace root plus
/// the state schema version. The caller passes the **canonicalized** root;
/// paths are hashed, never exposed in the protocol.
pub fn workspace_hash(canonical_root: &Path, schema_version: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(schema_version.to_le_bytes());
    hasher.update(canonical_root.as_os_str().as_encoded_bytes());
    hex(&hasher.finalize())
}

/// Resolved state directory for one workspace.
pub fn workspace_state_dir(canonical_root: &Path, schema_version: u64) -> PathBuf {
    default_state_dir()
        .join("funzzy")
        .join(WORKSPACES_DIR)
        .join(workspace_hash(canonical_root, schema_version))
}

/// Full state file path for one workspace.
pub fn state_file_path(canonical_root: &Path, schema_version: u64) -> PathBuf {
    workspace_state_dir(canonical_root, schema_version).join(STATE_FILE_NAME)
}

/// Result of loading a state file: the history plus an optional recovery
/// warning. A warning means the previous file was unusable and the store
/// recovered to empty; the watcher stays usable (contract §5).
#[derive(Debug)]
pub struct LoadOutcome {
    pub history: DurationHistory,
    pub warning: Option<String>,
}

/// Versioned on-disk shape. `schema` must equal [`STATE_SCHEMA_VERSION`];
/// bounds are re-enforced on decode so a hand-edited or corrupt file cannot
/// cause unbounded allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredState {
    schema: u64,
    profiles: Vec<StoredProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredProfile {
    signature: String,
    successes: Vec<u64>,
    failures: Vec<u64>,
    cancelled: usize,
    superseded: usize,
    timed_out: usize,
}

/// Bounded, atomic, single-writer state adapter (contract §5).
pub struct DurationStore {
    path: PathBuf,
}

impl DurationStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The resolved state file path; callers can surface it in diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads history from disk. Missing file → empty history, no warning.
    /// Corrupt/wrong-version/oversized file → quarantined (renamed aside)
    /// with one warning and empty recovery; the watcher stays usable.
    pub fn load(&self) -> LoadOutcome {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return LoadOutcome {
                    history: DurationHistory::new(),
                    warning: None,
                };
            }
            Err(error) => {
                return LoadOutcome {
                    history: DurationHistory::new(),
                    warning: Some(format!(
                        "funzzy: cannot read duration history '{}': {}; starting empty",
                        self.path.display(),
                        error
                    )),
                };
            }
        };

        if bytes.len() as u64 > MAX_STATE_BYTES {
            self.quarantine(&format!(
                "duration history '{}' exceeds {} bytes; quarantined and starting empty",
                self.path.display(),
                MAX_STATE_BYTES
            ));
            return LoadOutcome {
                history: DurationHistory::new(),
                warning: Some("duration history oversized; recovered empty".to_owned()),
            };
        }

        let stored: StoredState = match serde_json::from_slice(&bytes) {
            Ok(stored) => stored,
            Err(error) => {
                self.quarantine(&format!(
                    "duration history '{}' is corrupt ({}); quarantined and starting empty",
                    self.path.display(),
                    error
                ));
                return LoadOutcome {
                    history: DurationHistory::new(),
                    warning: Some("duration history corrupt; recovered empty".to_owned()),
                };
            }
        };

        if stored.schema != STATE_SCHEMA_VERSION {
            self.quarantine(&format!(
                "duration history '{}' has schema version {} (expected {}); quarantined and starting empty",
                self.path.display(),
                stored.schema,
                STATE_SCHEMA_VERSION
            ));
            return LoadOutcome {
                history: DurationHistory::new(),
                warning: Some(format!(
                    "duration history schema version {} unsupported; recovered empty",
                    stored.schema
                )),
            };
        }

        if stored.profiles.len() > MAX_PROFILES {
            self.quarantine(&format!(
                "duration history '{}' has {} profiles (limit {}); quarantined and starting empty",
                self.path.display(),
                stored.profiles.len(),
                MAX_PROFILES
            ));
            return LoadOutcome {
                history: DurationHistory::new(),
                warning: Some(
                    "duration history profile count exceeds bound; recovered empty".to_owned(),
                ),
            };
        }

        let snapshots: Vec<ProfileSnapshot> = stored
            .profiles
            .into_iter()
            .map(|profile| ProfileSnapshot {
                signature: crate::plan::ExecutionSignature(profile.signature),
                successes: profile.successes,
                failures: profile.failures,
                cancelled: profile.cancelled,
                superseded: profile.superseded,
                timed_out: profile.timed_out,
            })
            .collect();

        match DurationHistory::from_snapshot(snapshots) {
            Ok(history) => LoadOutcome {
                history,
                warning: None,
            },
            Err(error) => {
                self.quarantine(&format!(
                    "duration history '{}' violates retention bounds ({}); quarantined and starting empty",
                    self.path.display(),
                    error
                ));
                LoadOutcome {
                    history: DurationHistory::new(),
                    warning: Some(
                        "duration history violates retention bounds; recovered empty".to_owned(),
                    ),
                }
            }
        }
    }

    /// Atomically persists history: temp file in the same directory, fsync,
    /// then rename over the target. Single-writer policy: the watcher is the
    /// only writer; concurrent writers are not supported and would resolve to
    /// last-rename-wins without silent JSON corruption. No write happens
    /// inside the watched workspace because the path lives under XDG state.
    pub fn save(&self, history: &DurationHistory) -> Result<(), String> {
        let snapshots = history.snapshot();
        if snapshots.len() > MAX_PROFILES {
            return Err(format!(
                "refusing to persist {} profiles (limit {})",
                snapshots.len(),
                MAX_PROFILES
            ));
        }

        let stored = StoredState {
            schema: STATE_SCHEMA_VERSION,
            profiles: snapshots
                .into_iter()
                .map(|snapshot| StoredProfile {
                    signature: snapshot.signature.0,
                    successes: snapshot.successes,
                    failures: snapshot.failures,
                    cancelled: snapshot.cancelled,
                    superseded: snapshot.superseded,
                    timed_out: snapshot.timed_out,
                })
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&stored)
            .map_err(|error| format!("cannot serialize duration history: {error}"))?;

        let parent = self
            .path
            .parent()
            .ok_or_else(|| format!("state path '{}' has no parent", self.path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create state dir '{}': {error}", parent.display()))?;

        let temp_path = parent.join(format!(
            ".{}.tmp",
            self.path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "state".to_owned())
        ));

        let write_result = (|| -> Result<(), String> {
            let file = open_exclusive(&temp_path)?;
            write_all_fsync(&file, &json)?;
            fs::rename(&temp_path, &self.path).map_err(|error| {
                format!(
                    "cannot atomically replace '{}' with '{}': {error}",
                    temp_path.display(),
                    self.path.display()
                )
            })?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        write_result
    }

    /// Moves the current state file aside (quarantine) so recovery is
    /// observable and a bad file never blocks startup. Best-effort: if the
    /// rename fails the file stays put but the warning already fired.
    fn quarantine(&self, diagnostic: &str) {
        eprintln!("funzzy: {diagnostic}");
        let _ = fs::rename(&self.path, self.path.with_extension("corrupt"));
    }
}

#[cfg(unix)]
fn open_exclusive(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            format!(
                "cannot create state temp file '{}': {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
fn open_exclusive(path: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "cannot create state temp file '{}': {error}",
                path.display()
            )
        })
}

fn write_all_fsync(file: &fs::File, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = file;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write duration history: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot fsync duration history: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration_history::{ExcludedKind, SUCCESS_RETENTION};
    use crate::plan::ExecutionSignature;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn sig(n: u64) -> ExecutionSignature {
        ExecutionSignature(format!("sig-{n}"))
    }

    /// Unique temp dir per test; removed on drop.
    struct TempDir(PathBuf);
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    impl TempDir {
        fn new() -> Self {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "funzzy-duration-store-{}-{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn history_with_samples(count: usize) -> DurationHistory {
        let mut history = DurationHistory::new();
        let signature = sig(1);
        for sample in 1..=count {
            history.record_success(&signature, sample as u64 * 1_000);
        }
        history
    }

    #[test]
    fn missing_file_yields_empty_history_without_warning() {
        let temp = TempDir::new();
        let store = DurationStore::new(temp.0.join("run-durations-v1.json"));
        let outcome = store.load();
        assert!(outcome.warning.is_none());
        assert_eq!(outcome.history.success_samples(&sig(1)), 0);
    }

    #[test]
    fn save_then_load_round_trips_history() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let store = DurationStore::new(path.clone());
        let mut history = history_with_samples(3);
        history.record_failure(&sig(1), 500);
        history.record_excluded(&sig(1), ExcludedKind::Cancelled);
        history.record_excluded(&sig(1), ExcludedKind::Superseded);
        history.record_excluded(&sig(1), ExcludedKind::TimedOut);

        store.save(&history).expect("save succeeds");

        let outcome = DurationStore::new(path).load();
        assert!(outcome.warning.is_none());
        assert_eq!(
            outcome.history.estimate(&sig(1), None).unwrap().typical_ms,
            2_000
        );
        assert_eq!(outcome.history.success_samples(&sig(1)), 3);
        assert_eq!(outcome.history.failure_samples(&sig(1)), 1);
        assert_eq!(outcome.history.excluded_counts(&sig(1)), (1, 1, 1));
    }

    #[test]
    fn corrupt_file_is_quarantined_and_recovers_empty() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        fs::write(&path, b"{ not json ").unwrap();
        let store = DurationStore::new(path.clone());

        let outcome = store.load();
        assert!(outcome.warning.is_some());
        assert_eq!(outcome.history.success_samples(&sig(1)), 0);
        // Original file moved aside; a second load stays empty without a
        // second quarantine (file no longer exists).
        assert!(!path.exists());
        assert!(path.with_extension("corrupt").exists());
        let second = store.load();
        assert!(second.warning.is_none());
    }

    #[test]
    fn wrong_schema_version_is_quarantined() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        fs::write(
            &path,
            serde_json::to_vec(&StoredState {
                schema: 999,
                profiles: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        let outcome = DurationStore::new(path).load();
        assert!(outcome.warning.is_some());
        assert!(outcome
            .warning
            .as_ref()
            .unwrap()
            .contains("schema version 999"));
    }

    #[test]
    fn oversized_file_is_rejected_before_decode() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        fs::write(&path, vec![b'x'; (MAX_STATE_BYTES as usize) + 1]).unwrap();
        let outcome = DurationStore::new(path).load();
        assert!(outcome.warning.is_some());
        assert!(outcome.warning.as_ref().unwrap().contains("oversized"));
    }

    #[test]
    fn truncated_valid_json_is_quarantined() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let full = serde_json::to_vec(&StoredState {
            schema: STATE_SCHEMA_VERSION,
            profiles: vec![StoredProfile {
                signature: "sig-1".to_owned(),
                successes: vec![1_000, 2_000],
                failures: vec![],
                cancelled: 0,
                superseded: 0,
                timed_out: 0,
            }],
        })
        .unwrap();
        // Cut in the middle of the profiles array: valid JSON prefix, invalid
        // complete document.
        fs::write(&path, &full[..full.len() / 2]).unwrap();
        let outcome = DurationStore::new(path).load();
        assert!(outcome.warning.is_some());
        assert!(outcome.warning.as_ref().unwrap().contains("corrupt"));
    }

    #[test]
    fn permission_failure_warns_and_recovers_empty() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        fs::write(&path, b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            let outcome = DurationStore::new(path.clone()).load();
            assert!(outcome.warning.is_some());
            // Restore for cleanup.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(outcome.history.success_samples(&sig(1)) == 0);
        }
        #[cfg(not(unix))]
        {
            let _ = &path;
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_has_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let store = DurationStore::new(path.clone());
        store.save(&history_with_samples(2)).expect("save");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn atomic_replacement_leaves_no_temp_file() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let store = DurationStore::new(path.clone());
        store.save(&history_with_samples(2)).expect("first save");
        store.save(&history_with_samples(5)).expect("second save");

        assert!(path.exists());
        // No leftover temp artifacts in the parent dir.
        let leftovers: Vec<_> = fs::read_dir(temp.0.as_path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp leftovers: {leftovers:?}");
        // Reload reflects the newest content (last-rename-wins).
        let outcome = store.load();
        assert_eq!(outcome.history.success_samples(&sig(1)), 5);
    }

    #[test]
    fn save_rejects_oversized_profile_count() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let store = DurationStore::new(path);
        // MAX_PROFILES + 1 distinct signatures with one sample each.
        let mut history = DurationHistory::new();
        for n in 0..=MAX_PROFILES {
            history.record_success(&sig(n as u64), 1_000);
        }
        let error = store
            .save(&history)
            .expect_err("must reject oversized state");
        assert!(error.contains("refusing to persist"));
    }

    #[test]
    fn oversized_profile_snapshot_is_rejected_on_load() {
        let temp = TempDir::new();
        let path = temp.0.join("run-durations-v1.json");
        let snapshots = vec![ProfileSnapshot {
            signature: sig(1),
            successes: vec![0; SUCCESS_RETENTION + 1],
            failures: vec![],
            cancelled: 0,
            superseded: 0,
            timed_out: 0,
        }];
        let error = DurationHistory::from_snapshot(snapshots).expect_err("must reject");
        assert!(error.contains("retention bound"));
    }

    #[test]
    fn workspace_hash_is_stable_and_scoped() {
        let root = Path::new("/work/project");
        assert_eq!(
            workspace_hash(root, STATE_SCHEMA_VERSION),
            workspace_hash(root, STATE_SCHEMA_VERSION)
        );
        assert_ne!(
            workspace_hash(root, STATE_SCHEMA_VERSION),
            workspace_hash(Path::new("/work/other"), STATE_SCHEMA_VERSION)
        );
        assert_ne!(
            workspace_hash(root, STATE_SCHEMA_VERSION),
            workspace_hash(root, STATE_SCHEMA_VERSION + 1)
        );
    }

    #[test]
    fn state_file_path_is_outside_the_workspace() {
        let root = Path::new("/work/project");
        let path = state_file_path(root, STATE_SCHEMA_VERSION);
        assert!(
            !path.starts_with(root),
            "state must never live in the workspace"
        );
        assert!(path.to_string_lossy().contains("workspaces"));
        assert!(path.ends_with(STATE_FILE_NAME));
    }

    #[test]
    fn default_state_dir_honors_xdg_then_home_fallback() {
        let temp = TempDir::new();
        std::env::set_var("XDG_STATE_HOME", temp.0.join("xdg"));
        assert_eq!(
            default_state_dir(),
            temp.0.join("xdg"),
            "XDG_STATE_HOME wins"
        );
        std::env::remove_var("XDG_STATE_HOME");
        std::env::set_var("HOME", temp.0.join("home"));
        assert_eq!(
            default_state_dir(),
            temp.0.join("home").join(".local").join("state")
        );
    }
}
