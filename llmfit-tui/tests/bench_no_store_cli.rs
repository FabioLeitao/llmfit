use assert_cmd::Command;
use std::fs;
use std::io::{BufRead, BufReader, Error, ErrorKind, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OLLAMA_GENERATE_FIXTURE: &str = r#"{"response":"hi","eval_count":10,"eval_duration":2000000000,"prompt_eval_count":100,"prompt_eval_duration":500000000,"total_duration":3000000000}"#;

struct TempBenchStore {
    root: PathBuf,
}

impl TempBenchStore {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "llmfit-bench-store-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            nanos
        ));
        fs::create_dir_all(&root).expect("bench store root");
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempBenchStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn read_fixture_request(reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut reader = BufReader::new(reader);
    let mut request = Vec::new();
    let mut content_length = 0;
    loop {
        let start = request.len();
        if reader.read_until(b'\n', &mut request)? == 0 {
            return Err(Error::from(ErrorKind::UnexpectedEof));
        }
        let line = std::str::from_utf8(&request[start..])
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "fixture expects Content-Length, as sent by ureq::send_json",
                ));
            }
        }
    }
    let body_start = request.len();
    let request_length = body_start
        .checked_add(content_length)
        .ok_or_else(|| Error::from(ErrorKind::InvalidData))?;
    request.resize(request_length, 0);
    reader.read_exact(&mut request[body_start..])?;
    Ok(request)
}

/// Serve JSON on an ephemeral loopback port (warmup + runs for Ollama bench).
fn serve_ollama_generate_fixture(body: &'static str) -> String {
    use std::io::Write;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let addr = listener.local_addr().expect("test listener addr");
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set fixture read timeout");
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .expect("set fixture write timeout");
            read_fixture_request(&mut stream).expect("read fixture request");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write fixture response");
        }
    });
    format!("http://{}", addr)
}

fn run_ollama_bench_on_store(store: &Path, no_store: bool) -> Output {
    let base = serve_ollama_generate_fixture(OLLAMA_GENERATE_FIXTURE);
    let mut cmd = Command::cargo_bin("llmfit").expect("llmfit test binary");
    cmd.env("LLMFIT_BENCH_STORE", store)
        .env("NO_COLOR", "1")
        .arg("bench");
    if no_store {
        cmd.arg("--no-store");
    }
    cmd.args([
        "--provider",
        "ollama",
        "--url",
        &base,
        "--runs",
        "1",
        "--json",
        "fixture-model",
    ]);
    cmd.output().expect("failed to run llmfit")
}

fn pending_json_count(store: &Path) -> usize {
    let pending = store.join("pending");
    if !pending.is_dir() {
        return 0;
    }
    fs::read_dir(pending)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn bench_rejects_share_and_no_store_together() {
    let output = Command::cargo_bin("llmfit")
        .expect("llmfit test binary")
        .args(["bench", "--share", "--no-store", "--model", "llama3.2:3b"])
        .output()
        .expect("failed to run llmfit");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--no-store"),
        "expected --no-store in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("--share"),
        "expected --share in stderr, got: {stderr}"
    );
}

#[test]
fn bench_share_alone_does_not_require_no_store_flag() {
    let output = Command::cargo_bin("llmfit")
        .expect("llmfit test binary")
        .args(["bench", "--share", "--dry-run"])
        .output()
        .expect("failed to run llmfit");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cannot be used with --share while benchmarking"),
        "share-only path should not hit bench+no-store guard: {stderr}"
    );
    let _status: ExitStatus = output.status;
}

#[test]
fn bench_no_store_skips_pending_store() {
    let store = TempBenchStore::new();
    let output = run_ollama_bench_on_store(store.root(), true);

    assert!(
        output.status.success(),
        "bench failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        pending_json_count(store.root()),
        0,
        "expected no pending JSON with --no-store"
    );
}

#[test]
fn bench_without_no_store_writes_pending_store() {
    let store = TempBenchStore::new();
    let output = run_ollama_bench_on_store(store.root(), false);

    assert!(
        output.status.success(),
        "bench failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        pending_json_count(store.root()) >= 1,
        "expected at least one pending JSON without --no-store"
    );
}
