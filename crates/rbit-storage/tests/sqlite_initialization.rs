//! Exercise concurrent first use in fresh processes, independent of suite order.
use std::{
    process::Command,
    sync::{Arc, Barrier},
};

use sqlx::{Connection as _, SqliteConnection};

#[test]
fn concurrent_first_open_in_fresh_processes() {
    for _ in 0..4 {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "sqlite_initialization::fresh_process_worker", "--ignored", "--nocapture"])
            .output()
            .unwrap();
        assert!(output.status.success(), "fresh-process concurrent SQLite open failed: {}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "child test did not execute");
    }
}

#[test]
#[ignore = "invoked in fresh processes by concurrent_first_open_in_fresh_processes"]
fn fresh_process_worker() {
    let start = Arc::new(Barrier::new(32));
    std::thread::scope(|threads| {
        let workers: Vec<_> = (0..32)
            .map(|_| {
                let start = Arc::clone(&start);
                threads.spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                    start.wait();
                    runtime.block_on(async {
                        let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
                        let value: i64 = sqlx::query_scalar("SELECT 42").fetch_one(&mut connection).await.unwrap();
                        assert_eq!(value, 42);
                        connection.close().await.unwrap();
                    });
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
}
