//! D2 across processes: ten sync requests at once give one writer, and nine requests that return
//! within 50 ms having set the dirty flag. Each request is this test binary run again as a child.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cartograph_store::writer::{Exclusive, run_exclusive};

const ROLE: &str = "CARTOGRAPH_TEST_WRITER_DIR";

/// The child: waits for the start file, asks for a sync whose round takes 300 ms, and prints what
/// happened. Does nothing when run as an ordinary test.
#[test]
fn child_request() {
    let Ok(dir) = std::env::var(ROLE) else { return };
    let dir = Path::new(&dir);
    while !dir.join("go").exists() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let t = Instant::now();
    let out = run_exclusive::<_, std::io::Error>(dir, None, |_| {
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    })
    .unwrap();
    let us = t.elapsed().as_micros();
    match out {
        Exclusive::Busy => println!("\nRESULT busy {us}"),
        Exclusive::Ran { rounds, .. } => println!("\nRESULT ran {rounds} {us}"),
    }
}

#[test]
fn ten_requests_one_writer_nine_quick_exits() {
    let d = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = (0..10)
        .map(|_| {
            Command::new(&exe)
                .args([
                    "child_request",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(ROLE, d.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(d.path().join("go"), "").unwrap();
    let mut ran = Vec::new();
    let mut busy = Vec::new();
    for c in children {
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "a request exits 0");
        let text = String::from_utf8(out.stdout).unwrap();
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("RESULT "))
            .unwrap()
            .to_owned();
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "busy" => busy.push(f[1].parse::<u128>().unwrap()),
            _ => ran.push((f[1].parse::<u32>().unwrap(), f[2].parse::<u128>().unwrap())),
        }
    }
    eprintln!("writer (rounds, µs): {ran:?}; busy µs: {busy:?}");
    assert_eq!(ran.len(), 1, "exactly one writer");
    assert_eq!(busy.len(), 9);
    assert!(busy.iter().all(|&us| us < 50_000), "{busy:?}");
    // Every request raises the dirty flag before it tries the lock, as sync() does, so none is lost:
    // nine that landed before the first round began are covered by it (1 round), nine that landed
    // during it get exactly one more (2). Either way the flag is gone at the end.
    assert!(matches!(ran[0].0, 1 | 2), "rounds: {}", ran[0].0);
    assert!(!d.path().join("dirty").exists());
}
