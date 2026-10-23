//! The real binary: startup, `healthcheck` subcommand and graceful shutdown
//! on SIGTERM with an open request.

#![cfg(unix)]

use std::io::Write;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_lichtung-server");
const TOKEN: &str = "lt_process-test-bootstrap-token-0123456789";
/// An invented test key (base64 of 32 bytes).
const SECRET_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn healthcheck(port: u16) -> bool {
    // Only LICHTUNG_BIND: the check must work without the secrets.
    Command::new(BIN)
        .arg("healthcheck")
        .env_clear()
        .env("LICHTUNG_BIND", format!("0.0.0.0:{port}"))
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[test]
fn healthcheck_and_graceful_shutdown() {
    let port = free_port();
    assert!(!healthcheck(port), "nothing listens yet");

    let dir = tempfile::tempdir().unwrap();
    let mut server = Server(
        Command::new(BIN)
            .env("LICHTUNG_BIND", format!("127.0.0.1:{port}"))
            .env("LICHTUNG_DATA_DIR", dir.path().join("data"))
            .env("LICHTUNG_SECRET_KEY", SECRET_KEY)
            .env("LICHTUNG_BOOTSTRAP_TOKEN", TOKEN)
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    while !healthcheck(port) {
        assert!(Instant::now() < deadline, "server did not become healthy");
        std::thread::sleep(Duration::from_millis(100));
    }

    // Keep a request open (its body never completes); shutdown must still
    // complete.
    let mut open = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        open,
        "POST /api/tokens HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\n\
         Content-Type: application/json\r\nContent-Length: 1000\r\n\r\n{{"
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let pid = server.0.id().to_string();
    assert!(Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .unwrap()
        .success());
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "no graceful shutdown");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "exit status {status}");
    assert!(dir.path().join("data/lichtung.db").is_file());
    drop(open);
}

#[test]
fn invalid_configuration_fails_fast() {
    let output = Command::new(BIN)
        .env_clear()
        .env("LICHTUNG_SECRET_KEY", SECRET_KEY)
        .env("LICHTUNG_BACKUP_HOUR", "25")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("LICHTUNG_BACKUP_HOUR"));

    // Without the secret key (and without dev auth) the server refuses to
    // start.
    let output = Command::new(BIN).env_clear().output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("LICHTUNG_SECRET_KEY"));

    let output = Command::new(BIN).arg("unsinn").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_secret_key_that_does_not_match_the_database_stops_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let run = |key: &str, args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .env_clear()
            .env("LICHTUNG_BIND", format!("127.0.0.1:{}", free_port()))
            .env("LICHTUNG_DATA_DIR", &data)
            .env("LICHTUNG_SECRET_KEY", key)
            .env("RUST_LOG", "off")
            .output()
            .unwrap()
    };
    // Creates the database with a check value for SECRET_KEY.
    std::fs::create_dir_all(&data).unwrap();
    let output = run(SECRET_KEY, &["reset-secret-key"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 Konto"));

    // An invented, different key.
    let other = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
    let output = run(other, &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("LICHTUNG_SECRET_KEY passt nicht"),
        "{stderr}"
    );
    assert!(stderr.contains("reset-secret-key"), "{stderr}");
}
