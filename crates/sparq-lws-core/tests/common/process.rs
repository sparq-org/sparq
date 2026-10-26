//! Real native process plumbing shared by boot and durability tests.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Directory(pub PathBuf);

impl Directory {
    pub fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let dir = Self(std::env::temp_dir().join(format!("lws-native-{suffix}")));
        std::fs::create_dir(&dir.0).unwrap();
        dir
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Process {
    child: Option<Child>,
    log: Arc<Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Process {
    pub fn start(binary: &str, values: &[(&str, &str)]) -> Self {
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("SOLID_SERVER_BIND", "127.0.0.1:0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (key, value) in values {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&log);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n') {
                let Ok(mut line) = line else { break };
                line.push(b'\n');
                captured.lock().unwrap().extend(line);
            }
        });
        Self {
            child: Some(child),
            log,
            reader: Some(reader),
        }
    }

    pub fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    pub fn log(&self) -> String {
        String::from_utf8_lossy(&self.log.lock().unwrap()).into_owned()
    }

    pub async fn wait_ready(&mut self, client: &reqwest::Client, base: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "server exited before readiness: {}",
                self.log()
            );
            if let Ok(response) = client
                .get(format!("{base}/readyz"))
                .timeout(Duration::from_millis(250))
                .send()
                .await
            {
                if response.status().is_success() {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "server not ready: {}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(unix)]
    pub fn signal(&self, signal: &str) {
        assert!(Command::new("kill")
            .arg(signal)
            .arg(self.id().to_string())
            .status()
            .unwrap()
            .success());
    }

    #[cfg(unix)]
    pub fn stop(self) {
        self.signal("-TERM");
        let output = self.finish();
        assert_eq!(
            output.status.code(),
            Some(0),
            "SIGTERM: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    pub fn kill(self) {
        use std::os::unix::process::ExitStatusExt;
        self.signal("-KILL");
        assert_eq!(
            self.finish().status.signal(),
            Some(9),
            "must actually die by SIGKILL"
        );
    }

    pub fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "server did not exit: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut output = self.child.take().unwrap().wait_with_output().unwrap();
        self.reader.take().unwrap().join().unwrap();
        output.stderr = self.log.lock().unwrap().clone();
        output
    }

    // Boot refusals have no listener. The signal test observes registration in /proc instead.
    #[cfg(target_os = "linux")]
    pub fn wait_signal_handlers(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "{}",
                self.log()
            );
            let status = std::fs::read_to_string(format!("/proc/{}/status", self.id())).unwrap();
            let caught = status
                .lines()
                .find_map(|line| line.strip_prefix("SigCgt:\t"))
                .unwrap();
            let caught = u64::from_str_radix(caught.trim(), 16).unwrap();
            if caught & ((1 << 1) | (1 << 14)) == ((1 << 1) | (1 << 14)) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "signal handlers not registered: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
