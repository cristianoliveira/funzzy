//! Watch-session keyboard actions. The watch loop supplies input; this module
//! owns shortcut dispatch, busy-run latching, and clipboard feedback.

use crate::clipboard::{self, SystemClipboardWriter};
use crate::shortcut::{KeyDecode, TriggerLatch};
use crate::stdout;
use crate::watch_loop::RunStrategy;
use crate::watches::Watches;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualTriggerPhase {
    Idle,
    Waiting,
    Active(u64),
}

pub(crate) struct WatchShortcuts<'a> {
    strategy: &'a dyn RunStrategy,
    watches: &'a Arc<Mutex<Watches>>,
    latch: TriggerLatch,
    phase: ManualTriggerPhase,
}

impl<'a> WatchShortcuts<'a> {
    pub(crate) fn new(strategy: &'a dyn RunStrategy, watches: &'a Arc<Mutex<Watches>>) -> Self {
        Self {
            strategy,
            watches,
            latch: TriggerLatch::default(),
            phase: ManualTriggerPhase::Idle,
        }
    }

    pub(crate) fn handle(&mut self, key: Option<KeyDecode>) {
        match key {
            None => self.advance_manual_trigger(),
            Some(KeyDecode::CopyFailure) => self.copy_failure(),
            Some(KeyDecode::Eof) => {}
            Some(key) => self.accept_manual_trigger(key),
        }
    }

    fn advance_manual_trigger(&mut self) {
        if let ManualTriggerPhase::Active(generation) = self.phase {
            if self.strategy.is_generation_complete(generation) {
                self.phase = ManualTriggerPhase::Idle;
                self.latch.reset();
            }
        }
        if self.phase == ManualTriggerPhase::Waiting && !self.strategy.is_running() {
            self.run_manual();
        }
    }

    fn accept_manual_trigger(&mut self, key: KeyDecode) {
        if !self.latch.accept(key) {
            if key == KeyDecode::Trigger {
                stdout::info("Shortcut already latched; waiting for the current run to finish.");
            }
            return;
        }
        if self.strategy.is_running() {
            self.phase = ManualTriggerPhase::Waiting;
            stdout::info("Shortcut latched; waiting for the current run to finish.");
        } else {
            self.run_manual();
        }
    }

    fn run_manual(&mut self) {
        let (plan, revision) = {
            let watches = self.watches.lock().unwrap();
            (watches.manual_trigger_plan(), watches.revision().cloned())
        };
        let Some(plan) = plan else {
            stdout::warn("Shortcut ignored: no configured jobs to run.");
            self.latch.reset();
            self.phase = ManualTriggerPhase::Idle;
            return;
        };
        stdout::info("Running full pipeline from keyboard shortcut.");
        if let Some(generation) = self.strategy.run_manual(plan, revision) {
            self.phase = ManualTriggerPhase::Active(generation);
        } else {
            self.latch.reset();
            self.phase = ManualTriggerPhase::Idle;
        }
    }

    fn copy_failure(&self) {
        let foreground_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        match clipboard::copy_latest(
            &SystemClipboardWriter,
            foreground_tty,
            self.strategy.latest_failed_output(),
        ) {
            Ok(()) => stdout::info("Copied latest failed generation output to clipboard."),
            Err(message) => stdout::warn(&format!("Could not copy failure output: {message}")),
        }
    }
}
