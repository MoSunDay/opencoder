Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# 终端贪吃蛇示例 crate

- 新增完全独立的零依赖示例 `examples/snake`：纯 std 终端贪吃蛇，`stty raw -echo` 进入 raw mode、ANSI 转义原地重绘，alt-screen/回显在 Drop（含 panic unwind）恢复。
- 逻辑与 I/O 分离：`src/game.rs` 为纯状态机（单槽方向缓冲防 180° 掉头、尾部让位规则、LCG 播种食物、铺满判胜、`tick_ms` 随分数加速有下限），`src/main.rs` 只做输入线程、定时 tick 与渲染。
- 根 workspace 增加 `exclude = ["examples/snake"]`：workspace 成员、Cargo.lock 与主构建不受影响（`cargo metadata` 验证成员仍为 21 包）。

## 测试覆盖

| 行为 | 测试 | 入口 |
| --- | --- | --- |
| 掉头/同向输入被忽略，pending 二连按防 180° | `reverse_and_same_direction_are_ignored`、`pending_turn_blocks_quick_reverse` | `examples/snake/src/game.rs` |
| 撞墙死亡、死后 step 为 no-op | `wall_collision_ends_game` / `step_after_death_is_noop` | 同上 |
| 吃食加分变长、不吃保持长度、食物永不落在蛇身 | `eating_grows_and_scores` / `not_eating_keeps_length` / `food_never_spawns_on_snake` | 同上 |
| 尾部让位合法性（无食存活、有食致死） | `moving_into_tail_cell_is_legal` / `moving_into_tail_cell_with_food_is_fatal` | 同上 |
| 铺满棋盘判胜、tick 速度钳制无溢出 | `filling_the_board_wins` / `tick_speed_is_bounded` | 同上 |
| 无头冒烟：管道喂键干净退出、下行撞墙渲染 GAME OVER | exit=0，含 alt-screen 进/出序列与 `final score` | 本地冒烟 |

回归命令：

- `cargo test --manifest-path examples/snake/Cargo.toml`（12 项通过，0.00s）。
- `cargo clippy --manifest-path examples/snake/Cargo.toml --all-targets`（零警告）。
- `cargo metadata --no-deps --format-version 1`（workspace 结构未变）。

未做 workspace 全量回归：本轮未触碰任何 workspace crate，仅根 `Cargo.toml` 增加 exclude 一行，已由 metadata 验证。
