//! Terminal snake game. Zero dependencies: raw mode via `stty`,
//! rendering via ANSI escape codes.
//!
//! Run: cargo run --release
//! Keys: wasd / hjkl / arrows to move, r to restart, q (or ESC) to quit.

mod game;

use game::{tick_ms, Dir, Game};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

const W: i32 = game::DEFAULT_W;
const H: i32 = game::DEFAULT_H;

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    Left,
    Right,
    Restart,
    Quit,
}

/// Puts the terminal into raw/no-echo mode for its lifetime.
/// Restore also runs on panic unwind, so the shell is never left broken.
struct RawMode;

impl RawMode {
    fn enable() -> Self {
        stty(&["raw", "-echo"]);
        // Alternate screen + hide cursor.
        print!("\x1b[?1049h\x1b[?25l\x1b[2J");
        let _ = io::stdout().flush();
        RawMode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        print!("\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
        stty(&["sane"]);
    }
}

/// Apply stty settings on the controlling terminal; never fatal.
/// (When stdin is a pipe -- e.g. scripted smoke runs -- this simply fails.)
fn stty(args: &[&str]) {
    let mut cmd = std::process::Command::new("stty");
    cmd.args(args);
    if let Ok(tty) = std::fs::File::open("/dev/tty") {
        cmd.stdin(tty); // never touch our own stdin
    }
    let _ = cmd.status();
}

fn spawn_input() -> mpsc::Receiver<Key> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut buf = [0u8; 16];
        loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    for key in parse_keys(&buf[..n]) {
                        if tx.send(key).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    rx
}

fn parse_keys(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\x1b' if i + 2 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'O') => {
                match bytes[i + 2] {
                    b'A' => keys.push(Key::Up),
                    b'B' => keys.push(Key::Down),
                    b'C' => keys.push(Key::Right),
                    b'D' => keys.push(Key::Left),
                    _ => {}
                }
                i += 3;
                continue;
            }
            // Lone ESC or Ctrl-C.
            b'\x1b' | 0x03 => {
                keys.push(Key::Quit);
                return keys;
            }
            b'w' | b'W' | b'k' | b'K' => keys.push(Key::Up),
            b's' | b'S' | b'j' | b'J' => keys.push(Key::Down),
            b'a' | b'A' | b'h' | b'H' => keys.push(Key::Left),
            b'd' | b'D' | b'l' | b'L' => keys.push(Key::Right),
            b'r' | b'R' => keys.push(Key::Restart),
            b'q' | b'Q' => {
                keys.push(Key::Quit);
                return keys;
            }
            _ => {}
        }
        i += 1;
    }
    keys
}

fn border(w: i32) -> String {
    let mut s = String::with_capacity(w as usize + 2);
    s.push('+');
    s.push_str(&"-".repeat(w as usize));
    s.push('+');
    s
}

fn render(g: &Game) -> String {
    let mut grid = vec![b' '; g.w as usize * g.h as usize];
    for &(x, y) in g.snake.iter().skip(1) {
        grid[y as usize * g.w as usize + x as usize] = b'o';
    }
    if let Some(&(hx, hy)) = g.snake.front() {
        grid[hy as usize * g.w as usize + hx as usize] = b'O';
    }
    let (fx, fy) = g.food;
    grid[fy as usize * g.w as usize + fx as usize] = b'*';

    let mut out = String::with_capacity(4096);
    out.push_str("\x1b[H"); // cursor home, previous frame is overwritten in place
    out.push_str(&border(g.w));
    out.push('\n');
    for y in 0..g.h {
        out.push('|');
        for x in 0..g.w {
            out.push(grid[y as usize * g.w as usize + x as usize] as char);
        }
        out.push('|');
        out.push('\n');
    }
    out.push_str(&border(g.w));
    out.push_str("\n score: ");
    out.push_str(&g.score.to_string());
    out.push_str("   move: wasd/hjkl/arrows   r: restart   q: quit\n");
    if !g.alive {
        let title = if g.won { "YOU WIN" } else { "GAME OVER" };
        out.push_str(&format!(
            "\n ** {title} **   final score: {}\n [r] play again    [q] quit\n",
            g.score
        ));
    }
    out.push_str("\x1b[J"); // clear anything below
    out
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

fn main() {
    let mut game = Game::new(W, H, seed());
    let rx = spawn_input();
    let _raw = RawMode::enable();
    let mut stdout = io::stdout();

    loop {
        let timeout = if game.alive {
            Duration::from_millis(tick_ms(game.score))
        } else {
            Duration::from_millis(100)
        };
        match rx.recv_timeout(timeout) {
            Ok(Key::Quit) => break,
            Ok(Key::Restart) => game = Game::new(W, H, seed()),
            Ok(Key::Up) => game.turn(Dir::Up),
            Ok(Key::Down) => game.turn(Dir::Down),
            Ok(Key::Left) => game.turn(Dir::Left),
            Ok(Key::Right) => game.turn(Dir::Right),
            Err(RecvTimeoutError::Timeout) => {
                game.step();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let _ = write!(stdout, "{}", render(&game));
        let _ = stdout.flush();
    }
}
