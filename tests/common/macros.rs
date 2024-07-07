/// Defer a block of code to be executed when the scope is exited.
/// This is similar to the defer keyword in Go.
///
/// # Examples
///
/// ```
/// fn main() {
///    defer!({
///        println!("World");
///    });
///    println!("Hello");
/// }
/// // Output:
/// //   Hello
/// //   World
/// ```
#[macro_export]
macro_rules! defer {
    ($e:expr) => {
        struct ScopeCall<F: FnMut()> {
            c: F,
        }
        impl<F: FnMut()> Drop for ScopeCall<F> {
            fn drop(&mut self) {
                println!("Integration Tests: cleanup...");
                (self.c)();
            }
        }
        let _scope_call = ScopeCall {
            c: || -> () {
                $e;
            },
        };
    };
}

/// Wait until the expression is true.
///
/// On panic, this macro will print the values of the expressions with their
/// debug representations.
///
/// Like [`assert!`], this macro has a second form, where a custom
/// panic message can be provided.
///
/// # Examples
///
/// ```
/// wait_until!({
///   5 == 5
/// });
///
/// println!("success: 5 is equal to 5");
///
/// // Output:
/// //   success: 5 is equal to 5
///
/// wait_until!({
///   2 == 5
/// }, "2 is not equal to 5");
///
/// println!("success: 2 is equal to 5");
///
/// // Output:
/// //   panicked at '2 is not equal to 5'
///
/// wait_until!({
///   2 == 5
/// }, "2 is not equal to {}", 5);
///
/// println!("success: 2 is equal to 5");
///
/// // Output:
/// //   panicked at '2 is not equal to 5'
/// ```
#[macro_export]
macro_rules! wait_until {
    ($e:expr, $($arg:tt)+) => {
        for attempt in 0..200 {
            let result = $e;
            if result {
                break;
            }

            if attempt % 10 == 0 {
                println!("Integration Tests: waiting_until ...");
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        println!("Integration Tests: waiting_until timed out.");
        assert!($e, $($arg)+);
    };

    ($e:expr) => {
        for attempt in 0..100 {
            let result = $e;
            if result {
                break;
            }

            if attempt % 10 == 0 {
                println!("Integration Tests: waiting_until ...");
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert!($e, "Integration Tests: waiting_until timed out.");
    };
}

/// Write to a file.
/// it will write the string `test_content\n` to the given file for testing purposes.
#[macro_export]
macro_rules! write_to_file {
    ($file_path:expr) => {
        println!("Integration Tests: writting to file {}", $file_path);
        let mut file = match std::fs::File::create($file_path) {
            Ok(file) => file,
            Err(err) => {
                panic!(
                    "Integration Tests: [ERROR] failed to create file: {}\nCause: {:?}",
                    $file_path, err
                );
            }
        };

        match file.write_all(b"test_content\n") {
            Ok(_) => {}
            Err(err) => {
                panic!(
                    "Integration Tests: [ERROR] failed to write to file: {}\nCause: {:?}",
                    $file_path, err
                );
            }
        };
    };
}

/// Function to run commands in the shell
/// and print the output.
#[macro_export]
macro_rules! shell {
    ($cmd:expr) => {
        println!("---------FZZ tests log---------");
        println!("Integration Tests: running command: {}", $cmd);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg($cmd)
            .output()
            .expect("Integration Tests: failed to run command");

        println!(
            "Integration Tests: command output: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        println!("-------------------------------");
    };
}
