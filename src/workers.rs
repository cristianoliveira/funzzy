extern crate nix;

use crate::cmd::spawn;
use crate::rules::{self, Rules};
use crate::stdout;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    InitExecution,
    FinishedExecution(Duration),
    Tick,
}

pub struct Worker {
    canceller: Option<Sender<()>>,
    scheduler: Option<Sender<Vec<String>>>,

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn new(verbose: bool, fail_fast: bool, on_event: fn(WorkerEvent)) -> Self {
        stdout::verbose("Worker in verbose mode.", verbose);
        // Unfortunatelly channels can't have multiple receiver so we need to
        // create a channel for each kind of event.
        let (tscheduler, rscheduler) = channel::<Vec<String>>();
        let (tcancel, rcancel) = channel::<()>();

        let consumer = std::thread::spawn(move || {
            while let Ok(tasks) = rscheduler.recv() {
                on_event(WorkerEvent::InitExecution);
                let mut results: Vec<Result<(), String>> = vec![];
                let ignored = rcancel.try_recv();
                stdout::verbose(&format!("ignored kill: {:?}", ignored), verbose);

                let mut has_been_cancelled = false;
                let time_execution_started = std::time::Instant::now();

                for task in tasks {
                    if has_been_cancelled
                        || (fail_fast
                            && !results.clone().into_iter().find(|r| r.is_err()).is_none())
                    {
                        break;
                    }

                    let mut child = match spawn(&task) {
                        Ok(child) => child,
                        Err(err) => {
                            stdout::error(&format!("failed to create command: {:?}", err));
                            continue;
                        }
                    };

                    loop {
                        // We don't want the tasks to run in parallel but
                        // we need it to be async so we can kill it.
                        match child.try_wait() {
                            Ok(None) => {
                                // Task is still running...
                                // Check if there is any kill signal otherwise
                                // continue running
                                match rcancel.try_recv() {
                                    Ok(_) => {
                                        stdout::verbose(
                                            &format!("---- cancelling: {:?} ----", task),
                                            verbose,
                                        );

                                        if let Err(err) = signal::kill(
                                            Pid::from_raw(child.id() as i32),
                                            // Sends a SIGTERM signal to the process
                                            // and allows it to exit gracefully.
                                            Signal::SIGTERM,
                                        ) {
                                            stdout::error(&format!(
                                                "failed to terminate task {:?}: {:?}",
                                                task, err
                                            ));
                                        }

                                        if let Ok(status) = child.wait() {
                                            stdout::verbose(
                                                &format!(
                                                    "---- finished: {:?} status: {} ----",
                                                    task, status
                                                ),
                                                verbose,
                                            );
                                        } else {
                                            stdout::error(&format!(
                                                "failed to wait for the task to finish: {:?}",
                                                task
                                            ));
                                        }
                                        has_been_cancelled = true;
                                        break;
                                    }

                                    Err(err) if err != TryRecvError::Empty => {
                                        stdout::error(&format!(
                                            "failed to receive cancel event: {:?}",
                                            task
                                        ));
                                        break;
                                    }

                                    _ => {
                                        stdout::verbose(
                                            &format!("waiting next tick for task: {}", task),
                                            verbose,
                                        );

                                        on_event(WorkerEvent::Tick);

                                        std::thread::sleep(std::time::Duration::from_millis(200));
                                    }
                                }
                            }

                            Ok(Some(status)) => {
                                if status.success() {
                                    results.push(Ok(()));
                                } else {
                                    results.push(Err(format!(
                                        "Command {} has failed with {}",
                                        task, status
                                    )));
                                }

                                break;
                            }

                            Err(err) => {
                                results.push(Err(format!(
                                    "Command {} has errored with {}",
                                    task, err
                                )));

                                break;
                            }
                        };
                    }
                }

                if !has_been_cancelled {
                    let elapsed = time_execution_started.elapsed();
                    stdout::present_results(results, elapsed);
                    on_event(WorkerEvent::FinishedExecution(elapsed));
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            canceller: Some(tcancel),
            scheduler: Some(tscheduler),
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Some(canceller) = self.canceller.as_ref() {
            if let Err(err) = canceller.send(()) {
                println!("failed to send cancel signal: {:?}", err);
                return Err(format!("{:?}", err));
            }
        }

        Ok(())
    }

    pub fn schedule(&self, rules: Vec<Rules>, filepath: &str) -> Result<(), String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let current_dir = std::env::current_dir().unwrap();
            if let Err(err) = scheduler.send(rules::template(
                rules::commands(rules),
                rules::TemplateOptions {
                    filepath: Some(filepath.to_string()),
                    current_dir: format!("{}", current_dir.display()),
                },
            )) {
                return Err(format!("{:?}", err));
            }
        }

        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let tc = self.canceller.take();
        drop(tc);
        let ts = self.scheduler.take();
        drop(ts);
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}
