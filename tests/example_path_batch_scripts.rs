use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MANIFEST_ENV: &str = "FZZ_TRIGGER_PATHS_FILE";
const PATHS: [&str; 4] = [
    "input/one.txt",
    "input/two words.txt",
    "input/quote\"name.txt",
    "input/line\nbreak.txt",
];

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "funzzy-example-path-batch-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create isolated example directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ExampleRuntime {
    program: &'static str,
    script: &'static str,
}

const RUNTIMES: [ExampleRuntime; 2] = [
    ExampleRuntime {
        program: "node",
        script: "examples/changed-path-batch/node/print-changed-paths.mjs",
    },
    ExampleRuntime {
        program: "python3",
        script: "examples/changed-path-batch/python/print_changed_paths.py",
    },
];

fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn runtime_is_available(runtime: &ExampleRuntime) -> bool {
    Command::new(runtime.program)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run(runtime: &ExampleRuntime, working_directory: &Path, manifest: Option<&Path>) -> Output {
    let mut command = Command::new(runtime.program);
    command
        .arg(repository_path(runtime.script))
        .current_dir(working_directory)
        .env_remove(MANIFEST_ENV);

    if let Some(manifest) = manifest {
        command.env(MANIFEST_ENV, manifest);
    }

    command.output().expect("run changed-path example")
}

#[test]
fn node_and_python_examples_preserve_every_literal_manifest_path() {
    let directory = ScratchDirectory::new("literal-paths");
    let manifest = directory.path().join("changed-paths.json");
    std::fs::write(&manifest, serde_json::to_vec(&PATHS).unwrap()).unwrap();

    for runtime in &RUNTIMES {
        assert!(
            repository_path(runtime.script).is_file(),
            "missing {} example",
            runtime.script
        );
        if !runtime_is_available(runtime) {
            eprintln!("skipping optional {} example", runtime.program);
            continue;
        }

        let output = run(runtime, directory.path(), Some(&manifest));
        assert!(
            output.status.success(),
            "{} failed: {}",
            runtime.program,
            String::from_utf8_lossy(&output.stderr)
        );

        let observed = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<String>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(observed, PATHS, "{} changed path output", runtime.program);
    }
}

#[test]
fn node_and_python_examples_do_not_read_a_stale_file_on_pathless_runs() {
    let directory = ScratchDirectory::new("pathless");
    std::fs::write(
        directory.path().join("stale.json"),
        r#"["must-not-appear"]"#,
    )
    .unwrap();

    for runtime in &RUNTIMES {
        assert!(
            repository_path(runtime.script).is_file(),
            "missing {} example",
            runtime.script
        );
        if !runtime_is_available(runtime) {
            eprintln!("skipping optional {} example", runtime.program);
            continue;
        }

        let output = run(runtime, directory.path(), None);
        assert!(
            output.status.success(),
            "{} failed: {}",
            runtime.program,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "No changed-path batch (init or manual run).",
            "{} pathless output",
            runtime.program
        );
    }
}
