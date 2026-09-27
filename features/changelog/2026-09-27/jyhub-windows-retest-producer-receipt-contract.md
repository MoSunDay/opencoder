Commit: 6b5b4d28e047faa5b8d8468d6733c68cfd1e8685

# jyhub-windows-build 稳定性复验与 producer 回执契约收口（零代码改动）

## Context

operator skill 复验单：VideoFusion-win release/11.6.0@24190dc72a6863eae9f1289c7d2a7aa7be581895 走官方 full 出包、目标机验包、内容寻址上传与独立下载回读。执行期间 jyhub-windows-build skill 从 v2 演进到 v7，本轮是 windows-app 首次按 `finalize.py::validate_producer_receipt` 真实远端回读验收闭环。opencoder 仓库无代码改动，以下为外部工具链（workspace artifact-tools + 节点 skill + win2 构建机）的语义沉淀。

## 契约变更（沉淀为稳定语义）

- **源码锁查询位置**：远端分支 HEAD 只能在 operator Linux 侧带超时执行 `git ls-remote`；win2 出站 git over ssh 会长时间挂起，不在 win2 另做探查。以 `Switch-Release.ps1` 刷新的 `inputs/source-lock.json` 回执（commit/clean/lfs_verified）作为锁证据。
- **目标机验包脚本来源**：`verify_windows_bundle.ps1` 不在 Windows 产品源码仓内；权威路径 operator 侧 `/data00/workspace/agents/test-agent/artifact-tools/verify_windows_bundle.ps1`，先 scp 暂存到 win2 `C:\ProgramData\JyHubBuilder\tmp\`（哈希核对）再执行。不能因产品仓缺脚本跳过验包。
- **上传误报判定（v6）**：上传返回非零（complete 阶段 409 target already exists / 404 entry not found）时立即全量下载该远端路径核对 size+SHA-256；一致即记为 API 误报并继续，仅远端缺失或不一致才允许一次重试，禁止盲重试与覆盖 409。本轮 3 起（pdb.7z 409×3、JYInstaller.exe 404、主包 404）全部回读判真，零覆盖。
- **回执契约（v5/v7）**：`jy-builder-producer.v1` 必须能被 `agents/test-agent/artifact-tools/finalize.py::validate_producer_receipt` 直接消费：顶层 `schema`（键名非 format）、`source={"kind","version","branch"}` 三键、顶层数字 `build_exit_code`、`validations{package,target_runtime}`、`validation_evidence` 相对索引目录且文件真实存在（禁止列大包或不存在的路径）、每件 `artifacts` 含 name/remote/size/sha256 且 remote 含 sha256 前 20 位（内容寻址）。验收器对每件做独立远端下载回读（约 2.6GB，需以瞬态单元/后台通道承载，先本地结构预校验再触发）。

## 执行记录（证据：/data00/workspace/artifacts/jy-builder/2026-09-27/operator-skill-retest/）

- 构建：`win-20260927-0640-opfull`（build-all.cmd full，计划任务通道），06:42:26Z→07:18:08Z 共 2142s，exit_code=0，STAGE02-06 全 OK + `ALL DONE mode=full`。full 模式基线：VS2022 MSVC 14.44.35207、CMake 3.31.6-msvc6、Bun 1.3.14、ffmpeg 21.7.1。
- 运维事实：ssh 会话结束会杀掉 win2 上 `Start-Process` 拉起的后台进程（本轮验包/哈希首批即因此空跑）；win2 侧长任务必须经 `Submit-BuildStep.ps1` 计划任务通道承载。
- 验包：JYPacket 目录 `WorkSpace\bin\JYPacket\11.6.0.19072`，双 manifest 711+335 文件逐个哈希、5 个 PE AMD64、Bun/Lark 真实执行、addon 导出 26+10、App 存活 12s 全通过；主归档 `7z t` Everything is Ok（5134 文件）。Package 残留按 mtime 排除（`Jianying_...exe` 2026-09-21 旧残留豁免，未交付）。
- 交付：4 件制品 + BUILD_INFO.txt 内容寻址上传至 `/jyhub/VideoFusion-win/release-11.6.0/24190dc72a6863eae9f1289c7d2a7aa7be581895/<sha20>/`；5/5 独立回读逐字节一致；回执 `windows-app/receipt.json` 经验收器 ok=true。macos-app 旧回执同轮修正为同契约（source 三键、相对证据、BUILD_INFO 补传内容寻址）并通过验收。

## Impact / 兼容

- jy-builder producer skill（windows/macos）与 finalize 索引流后续按 v7 契约出回执；旧 `format` 键与含大包的 evidence 列表不再可消费。
- 大文件 win2→operator 传输用 python 分块 + ssh 管道（约 8MB/s，单连接易僵死需分块重试）；operator 侧上传/回读统一走 viking file-server 凭证。

## Related Docs

- [windows-operator-node 构建链路首次闭环](../2026-09-20/jyhub-windows-build-loop.md)
- [jy-builder team 上线](../2026-09-18/jy-builder-team-live.md)
- [jyhub macOS builder 部署](../2026-09-17/jyhub-macos-builder-deploy.md)
