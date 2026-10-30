use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MANIFEST: AtomicU64 = AtomicU64::new(1);
const CREATE_ATTEMPTS: usize = 128;

/// Owns one generation's changed-path manifest. The private directory and
/// read-only JSON file live outside the workspace and are removed when the
/// last supervised consumer releases the owner.
#[derive(Debug)]
pub(crate) struct TriggerPathsManifest {
    directory: PathBuf,
    path: PathBuf,
}

impl TriggerPathsManifest {
    pub(crate) fn create(paths: &[String]) -> Result<Self, String> {
        let root = absolute_temp_dir()?;
        Self::create_in(&root, paths)
    }

    fn create_in(root: &Path, paths: &[String]) -> Result<Self, String> {
        if paths.is_empty() {
            return Err("Cannot create changed-path manifest for an empty batch".to_owned());
        }

        let directory = create_private_directory(root)?;
        match write_manifest(&directory, paths) {
            Ok(path) => Ok(Self { directory, path }),
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                Err(error)
            }
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TriggerPathsManifest {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn absolute_temp_dir() -> Result<PathBuf, String> {
    let temp = std::env::temp_dir();
    if temp.is_absolute() {
        return Ok(temp);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(temp))
        .map_err(|error| format!("Cannot resolve changed-path manifest directory: {error}"))
}

fn create_private_directory(root: &Path) -> Result<PathBuf, String> {
    for _ in 0..CREATE_ATTEMPTS {
        let sequence = NEXT_MANIFEST.fetch_add(1, Ordering::Relaxed);
        let directory = root.join(format!(
            "funzzy-trigger-paths-{}-{sequence}",
            std::process::id()
        ));
        match create_directory(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Cannot create changed-path manifest directory '{}': {error}",
                    directory.display()
                ))
            }
        }
    }
    Err(format!(
        "Cannot create a unique changed-path manifest directory under '{}'",
        root.display()
    ))
}

#[cfg(unix)]
fn create_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

fn write_manifest(directory: &Path, paths: &[String]) -> Result<PathBuf, String> {
    let path = directory.join("paths.json");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|error| {
        format!(
            "Cannot create changed-path manifest '{}': {error}",
            path.display()
        )
    })?;
    serde_json::to_writer(&mut file, paths).map_err(|error| {
        format!(
            "Cannot encode changed-path manifest '{}': {error}",
            path.display()
        )
    })?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "Cannot write changed-path manifest '{}': {error}",
                path.display()
            )
        })?;
    drop(file);
    make_read_only(&path)?;

    let decoded: Vec<String> = serde_json::from_reader(File::open(&path).map_err(|error| {
        format!(
            "Cannot read changed-path manifest '{}': {error}",
            path.display()
        )
    })?)
    .map_err(|error| {
        format!(
            "Cannot verify changed-path manifest '{}': {error}",
            path.display()
        )
    })?;
    if decoded != paths {
        return Err(format!(
            "Changed-path manifest '{}' failed verification",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(unix)]
fn make_read_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).map_err(|error| {
        format!(
            "Cannot protect changed-path manifest '{}': {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn make_read_only(path: &Path) -> Result<(), String> {
    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("Cannot inspect changed-path manifest: {error}"))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("Cannot protect changed-path manifest: {error}"))
}

#[cfg(test)]
mod tests {
    use super::TriggerPathsManifest;
    use std::os::unix::fs::PermissionsExt;

    fn test_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "funzzy-trigger-paths-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn manifest_round_trips_literal_paths_with_private_read_only_permissions() {
        let root = test_root("round-trip");
        let paths = vec![
            "src/a b.rs".to_owned(),
            "src/quote'\"\t$x.rs".to_owned(),
            "src/line\nbreak.rs".to_owned(),
        ];

        let manifest = TriggerPathsManifest::create_in(&root, &paths).unwrap();
        let path = manifest.path().to_owned();
        let decoded: Vec<String> =
            serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();

        assert!(path.is_absolute());
        assert_eq!(decoded, paths);
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o400
        );

        drop(manifest);
        assert!(!path.exists(), "manifest must be removed with its owner");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_setup_failure_is_explicit_and_leaves_no_directory() {
        let root = test_root("failure");
        let not_a_directory = root.join("file");
        std::fs::write(&not_a_directory, "occupied").unwrap();

        let error = TriggerPathsManifest::create_in(&not_a_directory, &["a.rs".to_owned()])
            .expect_err("invalid manifest root must fail");

        assert!(error.contains("changed-path manifest"), "{error}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
