use cmd::spawn_command;
use std::sync::mpsc::channel;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use stdout;

#[derive(Debug, PartialEq, Eq)]
enum TaskEvent {
    Break = 1,
    Next = 2,
}

pub struct Worker {
    canceller: Sender<()>,
    scheduler: Sender<Vec<Vec<String>>>,
    dropper: Sender<()>,

    producer: Option<JoinHandle<()>>,
    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn new() -> Self {
        // Unfortunatelly channels can't have multiple receiver so we need to
        // create a channel for each kind of event.
        let (tscheduler, rscheduler) = channel::<Vec<Vec<String>>>();
        let (tconsumer, rconsumer) = channel::<TaskEvent>();
        let (tproducer, rproducer) = channel::<Option<Vec<String>>>();
        let (tcancel, rcancel) = channel::<()>();
        let (tdrop, rdrop) = channel::<()>();
        // producer
        let producer = std::thread::spawn(move || {
            while let Ok(mut rules) = rscheduler.recv() {
                if let Err(err) = tproducer.send(rules.pop()) {
                    stdout::error(&format!("failed to initiate the execution: {:?}", err));
                }

                while let Some(event) = rconsumer.recv().ok() {
                    if event == TaskEvent::Break {
                        break;
                    }

                    if let Err(err) = tproducer.send(rules.pop()) {
                        stdout::error(&format!("failed to initiate next execution: {:?}", err));
                    }
                }

                if let Ok(_) = rdrop.try_recv() {
                    if let Err(err) = tproducer.send(None) {
                        stdout::error(&format!("failed to finish last task: {:?}", err));
                    }
                    stdout::info(&format!("Killing all tasks..."));
                    break;
                }

                stdout::info(&format!("Finished running all tasks. Watching..."));
            }
        });

        // consumer
        let consumer = std::thread::spawn(move || {
            while let Ok(tasks_in_rule) = rproducer.recv() {
                // ignore first kill signal because it's the first task
                let _ = rcancel.try_recv();
                if let Some(tasks) = tasks_in_rule {
                    for task in tasks {
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
                                    if let Ok(_) = rcancel.try_recv() {
                                        stdout::info(&format!(
                                            "---- cancelling remaining tasks ----"
                                        ));

                                        if let Err(err) = child.kill() {
                                            stdout::error(&format!(
                                                "failed to kill current task: {:?}",
                                                err
                                            ));
                                        };

                                        if let Err(err) = tconsumer.send(TaskEvent::Break) {
                                            stdout::error(&format!(
                                                "failed to send stop current queued tasks: {:?}",
                                                err
                                            ));
                                        };

                                        break;
                                    }

                                    std::thread::sleep(std::time::Duration::from_millis(200));
                                    continue;
                                }
                                _ => {
                                    if let Err(err) = tconsumer.send(TaskEvent::Next) {
                                        stdout::error(&format!(
                                            "failed to request next task: {:?}",
                                            err
                                        ));
                                    };
                                }
                            };
                            break;
                        }
                    }
                } else {
                    if let Err(err) = tconsumer.send(TaskEvent::Break) {
                        stdout::error(&format!("failed to send final break: {:?}", err));
                    };
                }
            }
        });

        Worker {
            canceller: tcancel,
            scheduler: tscheduler,
            dropper: tdrop,
            producer: Some(producer),
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Err(err) = self.canceller.send(()) {
            return Err(format!("{:?}", err));
        }

        Ok(())
    }

    pub fn schedule(&self, rules: Vec<Vec<String>>) -> Result<(), String> {
        if let Err(err) = self.scheduler.send(rules) {
            return Err(format!("{:?}", err));
        }

        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.dropper.send(()).unwrap();
        if let Some(th) = self.producer.take() {
            th.join().expect("failed to join producer thread");
        }
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}
