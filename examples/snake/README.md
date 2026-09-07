# snake

Zero-dependency terminal snake game in Rust (std only: `stty` raw mode + ANSI escapes).

```bash
cd examples/snake
cargo run --release
```

Keys: `wasd` / `hjkl` / arrows to move, `r` restart, `q` / ESC quit.
Speed ramps up with the score; walls and your own body are fatal.
