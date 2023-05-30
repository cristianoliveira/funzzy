use cmd::spawn_command;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Sender, TryRecvError};
use std::thread::JoinHandle;
use stdout;

#[derive(Debug, PartialEq, Eq)]
enum TaskEvent {
    Break = 1,
    Next = 2,
    Kill = 3,
}

pub struct Worker {
    canceller: Option<Sender<()>>,
    scheduler: Option<Sender<Vec<String>>>,

    producer: Option<JoinHandle<()>>,
    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn new(verbose: bool) -> Self {
        if verbose {
            stdout::verbose(&format!("Worker in verbose mode."));
        };
        // Unfortunatelly channels can't have multiple receiver so we need to
        // create a channel for each kind of event.
        let (tscheduler, rscheduler) = channel::<Vec<String>>();
        let (tconsumer, rconsumer) = channel::<TaskEvent>();
        let (tproducer, rproducer) = channel::<Option<String>>();
        let (tcancel, rcancel) = channel::<()>();

        let producer = std::thread::spawn(move || {
            let mut thread_finished = false;
            while let Ok(mut tasks) = rscheduler.recv() {
                if verbose {
                    stdout::info(&format!("---- tasks scheduled {:?} ----", tasks));
                }

                if let Err(err) = tproducer.send(tasks.pop()) {
                    stdout::error(&format!("failed to initiate first task: {:?}", err));
                }

                // Block thread until all tasks are consumed or consumer is killed
                while let Some(event) = rconsumer.recv().ok() {
                    match event {
                        TaskEvent::Break => {
                            if verbose {
                                stdout::verbose(&format!("---- breaking ----"));
                            }
                            break;
                        }

                        TaskEvent::Next => {
                            if verbose {
                                stdout::verbose(&format!("---- next consumer ----"));
                            }
                            if let Err(err) = tproducer.send(tasks.pop()) {
                                stdout::error(&format!("failed to send next task: {:?}", err));
                            }
                        }

                        TaskEvent::Kill => {
                            if verbose {
                                stdout::verbose(&format!("---- kill consumer ----"));
                            }

                            thread_finished = true;

                            break;
                        }
                    }
                }

                if thread_finished {
                    break;
                }

                if verbose {
                    stdout::verbose(&format!(
                        "Finished producing tasks. Waiting new schedule..."
                    ));
                }
            }

            stdout::info(&format!("Producer thread finished."));
        });

        let consumer = std::thread::spawn(move || {
            while let Ok(next_task) = rproducer.recv() {
                let ignored = rcancel.try_recv();
                if verbose {
                    stdout::verbose(&format!("ignored kill: {:?}", ignored));
                }

                if let None = next_task {
                    if let Err(err) = tconsumer.send(TaskEvent::Break) {
                        stdout::error(&format!("failed to send final break: {:?}", err));
                    };

                    continue;
                }

                let task = next_task.unwrap();
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
                                    if verbose {
                                        stdout::info(&format!("---- cancelling: {:?} ----", task));
                                    }

                                    if let Err(err) = child.kill() {
                                        stdout::error(&format!(
                                            "failed to kill the task {:?}: {:?}",
                                            task, err
                                        ));
                                    }

                                    if let Ok(status) = child.wait() {
                                        if verbose {
                                            stdout::info(&format!(
                                                "---- finished: {:?} status: {} ----",
                                                task, status
                                            ));
                                        }
                                    } else {
                                        stdout::error(&format!(
                                            "failed to wait for the task to finish: {:?}",
                                            task
                                        ));
                                    }

                                    if let Err(err) = tconsumer.send(TaskEvent::Break) {
                                        stdout::error(&format!(
                                            "failed to send stop current queued tasks: {:?}",
                                            err
                                        ));
                                    };

                                    break;
                                }

                                Err(val) if val != TryRecvError::Empty => {
                                    if let Err(err) = tconsumer.send(TaskEvent::Kill) {
                                        stdout::error(&format!(
                                            "failed to send stop current queued tasks: {:?}",
                                            err
                                        ));
                                    };
                                }

                                _ => {
                                    if verbose {
                                        stdout::verbose(&format!(
                                            "waiting next tick for task: {}",
                                            task
                                        ));
                                    }

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

                            if let Err(err) = tconsumer.send(TaskEvent::Next) {
                                stdout::error(&format!("failed to request next task: {:?}", err));
                            };

                            break;
                        }
                        Err(err) => {
                            stdout::error(&format!("failed while trying to wait: {:?}", err));

                            if let Err(err) = tconsumer.send(TaskEvent::Next) {
                                stdout::error(&format!("failed to request next task: {:?}", err));
                            };

                            break;
                        }
                    };
                }
            }

            stdout::info(&format!("Consumer thread finished."));
        });

        Worker {
            canceller: Some(tcancel),
            scheduler: Some(tscheduler),
            producer: Some(producer),
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
        if let Some(th) = self.producer.take() {
            th.join().expect("failed to join producer thread");
        }
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}
