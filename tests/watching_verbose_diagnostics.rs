//! Behavior tests for typed lifecycle diagnostics (TASK-0023).
//!
//! Covers, against spawned binaries in verbose mode: startup records, event
//! batch records, matched/ignored/unmatched decisions with effective rule and
//! origin, scheduled generations, command/outcome records, the bounded
//! feedback-loop warning, and the guarantee that unrelated rapid events never
//! warn. Also proves normal (non-verbose) output stays free of debug records.

use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

use std::io::Seek;

use std::path::PathBuf;

/// Writes a config with the given content into a private scratch dir and
/// returns its absolute path plus the scratch dir.
fn scratch_config(label: &str, content: &str) -> (PathBuf, PathBuf) {
    let scratch =
        std::env::temp_dir().join(format!("funzzy-diag-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("failed to create scratch dir");
    let scratch = std::fs::canonicalize(&scratch).expect("failed to canonicalize scratch dir");
    let config_path = scratch.join("config.yml");
    std::fs::write(&config_path, content).expect("failed to write scratch config");
    (scratch, config_path)
}

fn read_log(output_log: &mut std::fs::File, output: &mut String) {
    output.clear();
    // The child appends to the same inode; rewind our handle so every read
    // sees the whole log regardless of the previous cursor position.
    output_log
        .seek(std::io::SeekFrom::Start(0))
        .expect("failed to rewind log");
    output_log
        .read_to_string(output)
        .expect("failed to read from file");
}

const BASE_CONFIG: &str = "
- name: docs
  run: 'echo building docs'
  change: 'examples/workdir/**/*.md'

- name: source
  run: 'echo building source'
  change: 'examples/workdir/**/*.rs'
  ignore: 'examples/workdir/generated/**'

- name: failing
  run: 'false'
  change: 'examples/workdir/**/*.fail'
";

#[test]
fn verbose_emits_startup_event_matched_and_outcome_records() {
    let (scratch, config_path) = scratch_config("lifecycle", BASE_CONFIG);
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_lifecycle.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=startup")
                },
                "startup record missing: {}",
                output
            );
            // Startup shows workspace, task count, busy policy, and roots.
            assert!(
                output.contains("tasks=3") && output.contains("policy=wait"),
                "startup summary incomplete: {}",
                output
            );
            assert!(
                output.contains(&format!("workspace={}", fixture.display())),
                "startup must name the workspace root: {}",
                output
            );

            output.clear();
            write_to_file!(fixture.join("examples/workdir/guide.md"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("state=passed")
                },
                "run outcome missing: {}",
                output
            );
            // Event record with batch/source/kind/path.
            assert!(
                output.contains("source=filesystem") && output.contains("decision=event"),
                "event record missing: {}",
                output
            );
            assert!(
                output.contains("examples/workdir/guide.md"),
                "event record must name the changed path: {}",
                output
            );
            // Matched decision names task, effective change rule, and origin.
            assert!(
                output.contains("decision=matched")
                    && output.contains("task=docs")
                    && output.contains("change=\"examples/workdir/**/*.md\"")
                    && output.contains("rule_origin=task"),
                "matched decision record missing: {}",
                output
            );
            // Scheduled run + command lifecycle.
            assert!(
                output.contains("decision=scheduled")
                    && output.contains("run=0")
                    && output.contains("policy=wait")
                    && output.contains("commands=1"),
                "scheduled run record missing: {}",
                output
            );
            assert!(
                output.contains("command=1/1 state=started command=\"echo building docs\""),
                "command started record missing: {}",
                output
            );
            assert!(
                output.contains("state=passed") && output.contains("duration="),
                "outcome record missing: {}",
                output
            );
            assert!(
                !output.contains("Events Ok"),
                "raw Debug dumps must be gone: {}",
                output
            );

            // Failure outcome: the failing task reports state=failed.
            output.clear();
            write_to_file!(fixture.join("examples/workdir/boom.fail"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("state=failed")
                },
                "failed outcome record missing: {}",
                output
            );
            assert!(
                output.contains("task=failing"),
                "failed decision must name the task: {}",
                output
            );
        },
    );
}

#[test]
fn verbose_explains_ignored_and_unmatched_paths_without_running() {
    let (scratch, config_path) = scratch_config("explain", BASE_CONFIG);
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_explain.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=startup")
                },
                "startup record missing: {}",
                output
            );

            // A path ignored by its only matching rule is explained as
            // ignored, naming the winning ignore rule and its origin.
            std::fs::create_dir_all(fixture.join("examples/workdir/generated"))
                .expect("create generated dir");
            output.clear();
            write_to_file!(fixture.join("examples/workdir/generated/out.rs"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=ignored")
                },
                "ignored decision missing: {}",
                output
            );
            assert!(
                output.contains("task=source")
                    && output.contains("ignore=\"examples/workdir/generated/**\"")
                    && output.contains("rule_origin=task"),
                "ignored decision must name rule and origin: {}",
                output
            );
            assert!(
                !output.contains("state=started"),
                "ignored paths must never execute work: {}",
                output
            );

            // A path matching nothing is an explicit unmatched decision. The
            // wait is path-specific: FSEvents can replay copied fixture files
            // at registration, so an early unmatched record alone is not proof.
            output.clear();
            write_to_file!(fixture.join("examples/workdir/notes.txt"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=unmatched") && output.contains("notes.txt")
                },
                "unmatched decision missing: {}",
                output
            );
        },
    );
}

#[test]
fn verbose_reports_group_rule_origin_for_inherited_patterns() {
    let (scratch, config_path) = scratch_config(
        "group-origin",
        "
on:
  change: 'examples/workdir/**/*.rs'
tasks:
  - name: backend-build
    run: 'echo backend'
",
    );
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_group_origin.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=startup")
                },
                "startup record missing: {}",
                output
            );

            output.clear();
            write_to_file!(fixture.join("examples/workdir/main.rs"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=matched")
                },
                "matched decision missing: {}",
                output
            );
            assert!(
                output.contains("task=backend-build") && output.contains("rule_origin=group"),
                "group-inherited rule must report group origin: {}",
                output
            );
        },
    );
}

#[test]
fn non_block_verbose_emits_generations_cancellations_and_loop_warning() {
    let (scratch, config_path) = scratch_config(
        "loop",
        "
- name: generate api
  run: 'echo generated >> examples/workdir/out.txt; sleep 2'
  change: 'examples/workdir/**'
",
    );
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_loop.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd
                .args(["-v", "--restart"])
                .spawn()
                .expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=startup") && output.contains("policy=restart")
                },
                "startup record missing: {}",
                output
            );

            // Seed the loop: one write schedules generation 1, whose command
            // appends to the watched file, so each debounce window re-triggers
            // the same task/path/rule chain.
            write_to_file!(fixture.join("examples/workdir/out.txt"));

            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("possible feedback loop")
                },
                "feedback loop warning missing: {}",
                output
            );
            assert!(
                output.contains("task=\"generate api\"")
                    && output.contains("repeats=")
                    && output.contains("hint="),
                "loop warning must name task, repeats, and hint: {}",
                output
            );
            assert!(
                output.contains("run=1") && output.contains("policy=restart"),
                "generation records missing: {}",
                output
            );
            // The replacement policy cancels the active run: a cancelled
            // record with a replacement reason must appear as the loop runs.
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("state=cancelled") && output.contains("reason=replaced")
                },
                "replacement cancellation record missing: {}",
                output
            );
            // A later trigger for the same task observes the previous run.
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("observed_after_run=")
                },
                "observed_after_run correlation missing: {}",
                output
            );
            assert!(
                output.contains("state=started command=\"echo generated >>"),
                "command started record missing: {}",
                output
            );
        },
    );
}

#[test]
fn unrelated_rapid_events_never_warn() {
    let (scratch, config_path) = scratch_config(
        "unrelated",
        "
- name: docs
  run: 'echo docs'
  change: 'examples/workdir/**/*.md'
",
    );
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_unrelated.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("decision=startup")
                },
                "startup record missing: {}",
                output
            );

            // Distinct paths, spaced beyond one debounce window so each is its
            // own batch: rapid unrelated events must never warn.
            for index in 0..4 {
                output.clear();
                write_to_file!(fixture.join(format!("examples/workdir/page-{}.md", index)));
                wait_until!(
                    {
                        read_log(&mut output_log, &mut output);
                        output.contains(&format!("page-{}.md", index))
                            && output.contains("decision=matched")
                    },
                    "event {} record missing: {}",
                    index,
                    output
                );
                std::thread::sleep(std::time::Duration::from_millis(1200));
            }

            read_log(&mut output_log, &mut output);
            assert!(
                !output.contains("possible feedback loop"),
                "unrelated rapid events must not warn: {}",
                output
            );
        },
    );
}

#[test]
fn normal_mode_output_has_no_debug_records() {
    let (scratch, config_path) = scratch_config("normal", BASE_CONFIG);
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "verbose_normal.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("Watching...")
                },
                "watcher not started: {}",
                output
            );

            write_to_file!(fixture.join("examples/workdir/guide.md"));
            wait_until!(
                {
                    read_log(&mut output_log, &mut output);
                    output.contains("Funzzy results")
                },
                "run results missing: {}",
                output
            );

            assert!(
                !output.contains("Funzzy debug:"),
                "normal mode must stay free of debug records: {}",
                output
            );
        },
    );
}
