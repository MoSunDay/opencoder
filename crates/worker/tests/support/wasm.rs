//! Wasm step fixtures for node-level tests: compile tiny WASI modules on
//! the fly and stage them into the node's shared module library
//! (`<data>/dag/_modules/<file>`). Staging there is race-free: the run's
//! own context root only exists after `Create` accepts the execution.

use std::path::Path;

/// Compile a WASI module that writes `message` (plus newline) to stdout.
fn stdout_module_wat(message: &str) -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{msg}\n")
  (func (export "_start")
    (i32.store (i32.const 64) (i32.const 0))
    (i32.store (i32.const 68) (i32.const {len}))
    (drop (call $fd_write (i32.const 1) (i32.const 64) (i32.const 1) (i32.const 72)))))"#,
        msg = message,
        len = message.len() + 1
    )
}

/// Stage `<data>/dag/_modules/<file>` printing `message` on `_start`.
pub fn stage_stdout_wasm(data_dir: &Path, file: &str, message: &str) {
    let library = data_dir.join("dag").join("_modules");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join(file),
        wat::parse_str(stdout_module_wat(message)).unwrap(),
    )
    .unwrap();
}

/// Stage `<data>/dag/_modules/spin.wasm` — an infinite loop ended only by
/// cancellation (epoch interruption) or timeout.
pub fn stage_spin_wasm(data_dir: &Path) {
    let library = data_dir.join("dag").join("_modules");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("spin.wasm"),
        wat::parse_str(r#"(module (func (export "_start") (loop $l (br $l))))"#).unwrap(),
    )
    .unwrap();
}
