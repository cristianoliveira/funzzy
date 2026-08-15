use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// TASK-0088/0090: a VALID config save no longer self-SIGTERMs; the watcher
/// stays alive in the same process. Two observables prove PID continuity:
/// - a no-op save (the example's init tasks touch the config file without
///   changing its content) reports no-op in-process (contract §3);
/// - a real semantic change hot-reloads to a new revision in-process.
/// The child process never exits on a valid save.
#[test]
fn valid_config_change_hot_reloads_without_process_exit() {
    setup::with_example(
        setup::Options {
            output_file: "valid_config_change_hot_reloads_without_process_exit.log",
            example_file: "examples/reload-config-example.yml",
        },
        |fzz_cmd, _output_log, fixture| {
            // The harness logs child stdout to `<output_file>-<pid>`. Re-read
            // it fresh on every tick instead of reusing the long-lived handle,
            // whose read cursor would freeze at EOF and miss later content.
            let log_path = std::env::current_dir()
                .expect("failed to get current dir")
                .join(format!(
                    "valid_config_change_hot_reloads_without_process_exit.log-{}",
                    std::process::id()
                ));
            let read_log = || std::fs::read_to_string(&log_path).unwrap_or_default();

            let mut child = fzz_cmd
                .arg("--restart")
                .spawn()
                .expect("failed to spawn child");

            // Phase 1: the example's init tasks touch the config file
            // (identical content) -> a no-op save handled in-process.
            wait_until!(
                { read_log().contains("Config save has no semantic change; nothing to reload.") },
                "no-op config save must be handled in-process, log:\n{}",
                read_log()
            );
            assert!(
                child.try_wait().expect("try_wait").is_none(),
                "no-op save must NOT terminate the watcher process"
            );

            // Phase 2: a real semantic change hot-reloads to a new revision
            // in the same process (contract §4 live point = atomic commit).
            std::fs::OpenOptions::new()
                .append(true)
                .open(fixture.join("examples/reload-config-example.yml"))
                .expect("failed to open fixture config")
                .write_all(
                    b"\n- name: hot reload proof\n  run: echo hot-reloaded\n  run_on_init: true\n",
                )
                .expect("failed to append semantic change");

            wait_until!(
                { read_log().contains("Config change is valid; hot-reloading") },
                "semantic config change must hot-reload, log:\n{}",
                read_log()
            );
            assert!(
                child.try_wait().expect("try_wait").is_none(),
                "hot reload must NOT terminate the watcher process"
            );
            assert!(!read_log().contains("Funzzy warning: unknown file/directory"));

            let _ = child.kill();
            let _ = child.wait();
        },
    );
}
