# feat(deploy): install.sh 新增 --backup 备份旧版（原子部署 + 回滚链路）

## 背景

`scripts/install.sh` 已实现原子部署（`cp`→`chmod`→`fsync`→`mv` rename，规避
ETXTBSY），但替换 `/usr/local/bin/opencoder` 前不留旧版本，回滚只能靠手动 `cp`
或重编译。需要一条可选的备份能力：部署前把现有目标保存为
`<dest>.bak.<时间戳>`，构成「原子部署 + 一键回滚」闭环。约束：默认行为零变化，
备份必须显式开启。

## 变更

### `scripts/install.sh`

- 新增 `--backup` 标志 + `OPENCODER_INSTALL_BACKUP` 环境变量（默认 `0`=关闭）。
  变量声明仿照 `DEST` 读取 `$OPENCODER_INSTALL_DEST` 的写法，解析分支新增
  `--backup) BACKUP=1; shift;;`。
- 在原子 rename 之前插入备份逻辑：仅当 `BACKUP != 0` 且 `$DEST` 已存在时执行；
  用 `[ "$DEST" -ef "$SRC" ]` 跳过「目标即源」（同 inode）的退化情形，避免备份
  即将安装的文件本身。
- 备份产物 `<dest>.bak.$(date +%Y%m%d%H%M%S)`，`cp -a` 保留权限/时间戳；失败
  复用退出码 `4`（install failed）。
- usage 文档与顶部用法示例注释同步更新（新增 `--backup` 行）。
- 不改原子 rename 主流程、版本自检、退出码表语义。

### `scripts/e2e/test_install.sh`

- 新增契约用例 C7 / C8（见下「测试覆盖」），文件头注释 C1..C8 齐全。

## 测试覆盖

新增契约测试 C7/C8（`scripts/e2e/test_install.sh`）：

| 功能 | 契约 | 文件 |
|------|------|------|
| `--backup` 把旧目标存为 `.bak.<ts>`，主目标更新为新版（内容各自正确） | C7 backup saves prior destination | [scripts/e2e/test_install.sh](../../../scripts/e2e/test_install.sh) |
| 目标不存在时 `--backup` 不产生备份、仍正常安装 | C8 no backup on fresh dest | [scripts/e2e/test_install.sh](../../../scripts/e2e/test_install.sh) |

## 全量回归

| 检查 | 结果 |
|------|------|
| `bash -n scripts/install.sh` | 语法 OK |
| `bash -n scripts/e2e/test_install.sh` | 语法 OK |
| `scripts/e2e/test_install.sh` | **8 passed / 0 failed**（含新增 C7/C8） |
| 说明 | 本变更仅触及 shell 部署脚本，不动 Rust 业务码，`cargo test` 基线不变 |

## Impact Surface

- 变更：[scripts/install.sh](../../../scripts/install.sh)（+~21 行，纯增量）、
  [scripts/e2e/test_install.sh](../../../scripts/e2e/test_install.sh)（+2 契约用例 + 头注释 2 行）。
- 不影响：任何 Rust crate、`Store`/`ChatStream` 抽象、session/web/cli/tui 业务行为；
  install.sh 默认行为向后兼容（`backup` 默认关闭）。
- 复用既有原子部署链路，备份仅是其前置步骤。

## 运维说明

```bash
scripts/install.sh --backup             # 部署 + 备份旧版
scripts/install.sh --backup --no-build  # 用已构建产物 + 备份
OPENCODER_INSTALL_BACKUP=1 scripts/install.sh
# 回滚（同样走原子 mv，避免 busy）：
mv -f /usr/local/bin/opencoder.bak.<ts> /usr/local/bin/opencoder
```
