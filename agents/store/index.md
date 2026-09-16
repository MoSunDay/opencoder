Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql/` — libsql 实现（WAL）

## 接缝
- `Arc<dyn Store>`：session/web/todos/control 等全部经此持久化。
