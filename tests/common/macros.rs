#[macro_export]
macro_rules! defer {
    ($e:expr) => {
        struct ScopeCall<F: FnMut()> {
            c: F,
        }
        impl<F: FnMut()> Drop for ScopeCall<F> {
            fn drop(&mut self) {
                println!("Cleaning up...");
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

#[macro_export]
macro_rules! wait_until {
    ($e:expr,$m:expr) => {
        for _ in 0..100 {
            let result = $e;
            if result {
                break;
            }
            sleep(Duration::from_millis(100));
        }

        assert!($e, $m);
    };

    ($e:expr) => {
        for _ in 0..100 {
            let result = $e;
            if result {
                break;
            }
            sleep(Duration::from_millis(100));
        }
    };
}

#[macro_export]
macro_rules! write_to_file {
    ($file:expr) => {
        let mut file = File::create($file).expect("failed to open file");
        file.write_all(b"test_content\n").expect("failed to write to file");
    };
}
