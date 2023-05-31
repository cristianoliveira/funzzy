use cmd::spawn_command;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Sender, TryRecvError};
use std::thread::JoinHandle;
use stdout;

pub struct Worker {
    canceller: Option<Sender<()>>,
    scheduler: Option<Sender<Vec<String>>>,

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn new(verbose: bool) -> Self {
        stdout::verbose(&format!("Worker in verbose mode."), verbose);
        // Unfortunatelly channels can't have multiple receiver so we need to
        // create a channel for each kind of event.
        let (tscheduler, rscheduler) = channel::<Vec<String>>();
        let (tcancel, rcancel) = channel::<()>();

        let consumer = std::thread::spawn(move || {
            while let Ok(mut tasks) = rscheduler.recv() {
                let ignored = rcancel.try_recv();
                stdout::verbose(&format!("ignored kill: {:?}", ignored), verbose);

                let mut has_been_cancelled = false;
                while let Some(task) = tasks.pop() {
                    if has_been_cancelled {
                        break;
                    }
                    stdout::info(&format!("---- running: {:?} ----", task));
                    let mut child = match spawn_command(task.clone()) {
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

                                        if let Err(err) = child.kill() {
                                            stdout::error(&format!(
                                                "failed to kill the task {:?}: {:?}",
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

                                        std::thread::sleep(std::time::Duration::from_millis(200));
                                    }
                                }
                            }
                            Ok(Some(status)) => {
                                if verbose {
                                    if status.success() {
                                        stdout::info(&format!("---- finished: {:?} ----", task));
                                    } else {
                                        stdout::error(&format!("---- failed: {:?} ----", task));
                                    }
                                }

                                break;
                            }
                            Err(err) => {
                                stdout::error(&format!("failed while trying to wait: {:?}", err));
                                break;
                            }
                        };
                    }
                }
            }

            stdout::info(&format!("Consumer thread finished."));
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

    pub fn schedule(&self, rules: Vec<Vec<String>>) -> Result<(), String> {
        let tasks = rules
            .iter()
            .map(|rule| rule.iter().cloned().collect::<Vec<String>>())
            .flatten()
            .collect::<Vec<String>>();

        if let Some(scheduler) = self.scheduler.as_ref() {
            if let Err(err) = scheduler.send(tasks) {
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
