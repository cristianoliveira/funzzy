//! Pure keyboard decoding and one-shot trigger policy for the watch loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// Ctrl-G is deliberately not Ctrl-C: SIGINT remains owned by the normal
/// shutdown path. It is a single byte in both raw terminals and pipes.
pub const TRIGGER_KEY: u8 = 0x07;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyDecode {
    Trigger,
    Ignore,
    Partial,
    Eof,
}

/// Decodes one input read without terminal or process side effects.
/// Empty non-EOF reads are retained as partial input for readers that return
/// short chunks; unknown bytes and unsupported escape sequences are ignored.
pub fn decode_key(bytes: &[u8], eof: bool) -> KeyDecode {
    if eof {
        return KeyDecode::Eof;
    }
    match bytes {
        [] | [0x1b] | [0x1b, b'['] => KeyDecode::Partial,
        [TRIGGER_KEY] => KeyDecode::Trigger,
        _ => KeyDecode::Ignore,
    }
}

/// Latches at most one shortcut request until the watch loop completes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TriggerLatch {
    pending: bool,
}

impl TriggerLatch {
    pub fn accept(&mut self, decoded: KeyDecode) -> bool {
        if decoded != KeyDecode::Trigger || self.pending {
            return false;
        }
        self.pending = true;
        true
    }

    pub fn reset(&mut self) {
        self.pending = false;
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }
}

fn stdin_readable() -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return true;
        }
        let fd = std::os::fd::AsRawFd::as_raw_fd(&stdin);
        // Reading a terminal from a background process group causes SIGTTIN.
        unsafe { nix::libc::tcgetpgrp(fd) == nix::libc::getpgrp() }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Starts a detached stdin reader. TTY input is put in noncanonical mode so
/// the key is delivered immediately; piped input is read normally. Ctrl-C is
/// left as SIGINT (`ISIG` remains enabled), and the original TTY settings are
/// restored when the reader exits.
pub fn start_reader(shutdown: Option<Arc<AtomicBool>>) -> Receiver<KeyDecode> {
    let (sender, receiver) = mpsc::channel();
    // A TTY inherited by a background process group is not safe to read:
    // the kernel sends SIGTTIN and stops the whole process group. This is
    // common when tests (or supervisors) spawn fzz with the parent's TTY.
    if !stdin_readable() {
        return receiver;
    }
    std::thread::spawn(move || {
        let terminal = RawTerminal::enter();
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut byte = [0_u8; 1];
        loop {
            if shutdown
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                break;
            }
            #[cfg(unix)]
            if terminal.is_some() {
                let mut poll = nix::libc::pollfd {
                    fd: std::os::fd::AsRawFd::as_raw_fd(&input),
                    events: nix::libc::POLLIN,
                    revents: 0,
                };
                let ready = unsafe { nix::libc::poll(&mut poll, 1, 100) };
                if ready <= 0 {
                    continue;
                }
            }
            match std::io::Read::read(&mut input, &mut byte) {
                Ok(0) => {
                    let _ = sender.send(KeyDecode::Eof);
                    break;
                }
                Ok(_) => {
                    if sender.send(decode_key(&byte, false)).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        drop(terminal);
    });
    receiver
}

struct RawTerminal {
    #[cfg(unix)]
    fd: std::os::fd::RawFd,
    #[cfg(unix)]
    original: nix::libc::termios,
}

impl RawTerminal {
    fn enter() -> Option<Self> {
        #[cfg(unix)]
        {
            use std::io::IsTerminal;
            let stdin = std::io::stdin();
            if !stdin.is_terminal() {
                return None;
            }
            let fd = std::os::fd::AsRawFd::as_raw_fd(&stdin);
            let mut original = std::mem::MaybeUninit::<nix::libc::termios>::uninit();
            if unsafe { nix::libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
                return None;
            }
            let original = unsafe { original.assume_init() };
            let mut raw = original;
            raw.c_lflag &= !(nix::libc::ICANON | nix::libc::ECHO);
            raw.c_cc[nix::libc::VMIN] = 1;
            raw.c_cc[nix::libc::VTIME] = 0;
            if unsafe { nix::libc::tcsetattr(fd, nix::libc::TCSANOW, &raw) } != 0 {
                return None;
            }
            Some(Self { fd, original })
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            let _ = nix::libc::tcsetattr(self.fd, nix::libc::TCSANOW, &self.original);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_key, KeyDecode, TriggerLatch, TRIGGER_KEY};

    #[test]
    fn decodes_only_the_documented_single_key() {
        assert_eq!(decode_key(&[TRIGGER_KEY], false), KeyDecode::Trigger);
        assert_eq!(decode_key(b"x", false), KeyDecode::Ignore);
        assert_eq!(decode_key(&[0x1b], false), KeyDecode::Partial);
        assert_eq!(decode_key(&[0x1b, b'['], false), KeyDecode::Partial);
        assert_eq!(decode_key(&[0x1b, b'[', b'A'], false), KeyDecode::Ignore);
    }

    #[test]
    fn empty_reads_are_partial_and_eof_is_terminal() {
        assert_eq!(decode_key(&[], false), KeyDecode::Partial);
        assert_eq!(decode_key(&[], true), KeyDecode::Eof);
    }

    #[test]
    fn latch_accepts_one_trigger_until_reset() {
        let mut latch = TriggerLatch::default();
        assert!(latch.accept(KeyDecode::Trigger));
        assert!(!latch.accept(KeyDecode::Trigger));
        assert!(!latch.accept(KeyDecode::Ignore));
        assert!(latch.is_pending());
        latch.reset();
        assert!(latch.accept(KeyDecode::Trigger));
    }
}
