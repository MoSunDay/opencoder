//! Pure game logic for the terminal snake.
//!
//! No I/O, no clocks: everything is a plain state transition so it can be
//! unit-tested in milliseconds.

use std::collections::VecDeque;

pub const DEFAULT_W: i32 = 30;
pub const DEFAULT_H: i32 = 18;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Debug)]
pub struct Game {
    pub w: i32,
    pub h: i32,
    /// Head is the front of the queue.
    pub snake: VecDeque<(i32, i32)>,
    pub dir: Dir,
    pub food: (i32, i32),
    pub score: u32,
    pub alive: bool,
    pub won: bool,
    pending: Option<Dir>,
    rng: u64,
}

impl Game {
    /// Standard game: snake of length 3 in the middle of the board, heading right.
    pub fn new(w: i32, h: i32, seed: u64) -> Self {
        assert!(w >= 5 && h >= 5, "board too small");
        let (cx, cy) = (w / 2, h / 2);
        let snake = [(cx, cy), (cx - 1, cy), (cx - 2, cy)].into_iter().collect();
        Self::with_body(w, h, seed, snake, Dir::Right)
    }

    /// Full control over the initial body; used by small boards and tests.
    pub fn with_body(w: i32, h: i32, seed: u64, snake: VecDeque<(i32, i32)>, dir: Dir) -> Self {
        let mut game = Game {
            w,
            h,
            snake,
            dir,
            food: (0, 0),
            score: 0,
            alive: true,
            won: false,
            pending: None,
            rng: seed | 1,
        };
        game.place_food();
        game
    }

    /// Queue a turn for the next step. Reversing into the snake's neck
    /// (and no-ops) are ignored; the comparison is against the pending
    /// turn so two quick presses cannot trigger a 180-degree flip.
    pub fn turn(&mut self, dir: Dir) {
        if !self.alive {
            return;
        }
        let current = self.pending.unwrap_or(self.dir);
        if dir == current || dir == current.opposite() {
            return;
        }
        self.pending = Some(dir);
    }

    /// Advance one tick. Returns true when the snake ate the food.
    pub fn step(&mut self) -> bool {
        if !self.alive {
            return false;
        }
        if let Some(dir) = self.pending.take() {
            self.dir = dir;
        }
        let head = self.snake.front().copied().unwrap_or((0, 0));
        let (dx, dy) = self.dir.delta();
        let next = (head.0 + dx, head.1 + dy);

        if next.0 < 0 || next.1 < 0 || next.0 >= self.w || next.1 >= self.h {
            self.alive = false;
            return false;
        }
        let ate = next == self.food;
        // Without growth the tail vacates its cell, so following it is legal.
        let solid = if ate {
            self.snake.len()
        } else {
            self.snake.len().saturating_sub(1)
        };
        if self.snake.iter().take(solid).any(|&p| p == next) {
            self.alive = false;
            return false;
        }

        self.snake.push_front(next);
        if ate {
            self.score += 1;
            self.place_food();
            true
        } else {
            self.snake.pop_back();
            false
        }
    }

    /// Respawn food on a free cell, or declare victory when none is left.
    pub fn place_food(&mut self) {
        let cells = self.w as usize * self.h as usize;
        if self.snake.len() >= cells {
            self.won = true;
            self.alive = false;
            return;
        }
        loop {
            self.rng = self
                .rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let x = ((self.rng >> 33) % self.w as u64) as i32;
            let y = ((self.rng >> 13) % self.h as u64) as i32;
            if !self.snake.contains(&(x, y)) {
                self.food = (x, y);
                return;
            }
        }
    }
}

/// Frame duration in ms: starts relaxed, ramps up with the score.
pub fn tick_ms(score: u32) -> u64 {
    const BASE: u64 = 140;
    const STEP: u64 = 5;
    const MIN: u64 = 60;
    BASE.saturating_sub(STEP * score as u64).max(MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(cells: &[(i32, i32)]) -> VecDeque<(i32, i32)> {
        cells.iter().copied().collect()
    }

    #[test]
    fn reverse_and_same_direction_are_ignored() {
        let mut g = Game::new(10, 10, 7);
        g.turn(Dir::Left); // opposite of the initial Right
        g.step();
        assert_eq!(g.dir, Dir::Right);
        let x = g.snake.front().unwrap().0;
        g.turn(Dir::Right); // same direction
        g.step();
        assert_eq!(g.dir, Dir::Right);
        assert_eq!(g.snake.front().unwrap().0, x + 1);
    }

    #[test]
    fn perpendicular_turn_applies_on_next_step() {
        let mut g = Game::new(10, 10, 3);
        g.turn(Dir::Up);
        let y = g.snake.front().unwrap().1;
        g.step();
        assert_eq!(g.dir, Dir::Up);
        assert_eq!(g.snake.front().unwrap().1, y - 1);
    }

    #[test]
    fn pending_turn_blocks_quick_reverse() {
        let mut g = Game::new(10, 10, 11);
        g.turn(Dir::Up);
        g.turn(Dir::Down); // opposite of the pending Up, must be dropped
        g.step();
        assert_eq!(g.dir, Dir::Up);
    }

    #[test]
    fn wall_collision_ends_game() {
        let mut g = Game::with_body(10, 10, 5, body(&[(8, 4), (7, 4), (6, 4)]), Dir::Right);
        g.food = (0, 0);
        g.step(); // head to (8+1=9, 4)
        assert!(g.alive);
        g.step(); // next would be (10, 4)
        assert!(!g.alive);
    }

    #[test]
    fn eating_grows_and_scores() {
        let mut g = Game::new(10, 10, 13);
        let len0 = g.snake.len();
        let head = g.snake.front().copied().unwrap();
        g.food = (head.0 + 1, head.1);
        assert!(g.step());
        assert_eq!(g.score, 1);
        assert_eq!(g.snake.len(), len0 + 1);
        assert!(!g.snake.contains(&g.food)); // respawned off the body
    }

    #[test]
    fn not_eating_keeps_length() {
        let mut g = Game::new(10, 10, 17);
        let len0 = g.snake.len();
        g.food = (0, 0); // far from the middle where the snake moves
        assert!(!g.step());
        assert_eq!(g.snake.len(), len0);
    }

    #[test]
    fn moving_into_tail_cell_is_legal() {
        // head (3,3) heading Down, tail (3,4): the tail vacates first.
        let mut g = Game::with_body(10, 10, 1, body(&[(3, 3), (2, 3), (2, 4), (3, 4)]), Dir::Down);
        g.food = (0, 0);
        g.step();
        assert!(g.alive);
        assert_eq!(g.snake.front(), Some(&(3, 4)));
    }

    #[test]
    fn moving_into_tail_cell_with_food_is_fatal() {
        let mut g = Game::with_body(10, 10, 1, body(&[(3, 3), (2, 3), (2, 4), (3, 4)]), Dir::Down);
        g.food = (3, 4); // eating keeps the tail in place -> real collision
        g.step();
        assert!(!g.alive);
    }

    #[test]
    fn food_never_spawns_on_snake() {
        for seed in 0..64u64 {
            let mut g = Game::new(12, 9, seed);
            assert!(!g.snake.contains(&g.food), "seed {seed}");
            for _ in 0..40 {
                g.turn(Dir::Down);
                g.step();
                g.turn(Dir::Right);
                g.step();
                g.turn(Dir::Up);
                g.step();
                if !g.alive {
                    break;
                }
                assert!(!g.snake.contains(&g.food), "seed {seed}");
            }
        }
    }

    #[test]
    fn filling_the_board_wins() {
        let mut g = Game::with_body(1, 2, 5, body(&[(0, 0)]), Dir::Down);
        g.food = (0, 1);
        g.step();
        assert!(g.won);
        assert!(!g.alive);
        assert_eq!(g.score, 1);
    }

    #[test]
    fn step_after_death_is_noop() {
        let mut g = Game::with_body(10, 10, 2, body(&[(9, 5), (8, 5), (7, 5)]), Dir::Right);
        g.step();
        assert!(!g.alive);
        let len = g.snake.len();
        g.step();
        assert_eq!(g.snake.len(), len);
    }

    #[test]
    fn tick_speed_is_bounded() {
        assert_eq!(tick_ms(0), 140);
        assert_eq!(tick_ms(100), 60);
        assert_eq!(tick_ms(u32::MAX), 60); // no overflow panic
    }
}
