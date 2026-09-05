Commit: 09118a1（工作树态：opencoder-agents crate + core 读路径 + session/cli/web/SPA 接线，未提交）

# opencoder-agents — 版本化自定义 Agent（写路径 + NFS）

读路径在 core（[agents/core](../core/index.md) `agent::{meta,resource,compose}`），本 crate 只做**写路径**与 **NFS 导出**，全部纯函数 + 数据 struct。

## 目录与版本模型（单一真源）

```
<agents_root>/            # ~/.opencoder/agents；set_agents_dir_override > OPENCODER_AGENTS_DIR > global home
├── active                # 单行 marker = 生效 agent 名（envs 同款：temp+fsync+rename、0o600、preflight 回滚）
├── prompts/<名>/{meta.json, v1|v2…/{soul,how,output}.md}   # 共享提示词包
├── skills/<名>/{meta.json, v1…/<skill>/SKILL.md}            # 共享技能集
├── tools/<名>/{meta.json, v1…/<可执行>}                     # 共享工具集（整目录注入 PATH）
├── memory/<名>/{meta.json, v1…/memory.md}
└── <agent>/meta.json     # 引用卡：current:{prompt,skills,tools,memory: 资源名} + history + references 快照
```

- **共享优先**：资源是顶层一等实体，多 agent 引用同一份；prompt 升 v2 = 所有引用它的 agent 同步生效。版本号只增不复用（next = max(history∪{current})+1）；**回滚 = 切 current 指针，不删历史**。
- 引用卡四字段均可缺省但 `current.prompt` 必须可解析才算可 resolve 的 agent（web 激活 preflight 据此拒绝无 prompt 卡：400 + marker 回滚）；agent 名保留字：`active/prompts/skills/tools/memory`。
- 资源 meta：`{name, created_at, updated_at, current: u32(0=无), history: [u32]}`；marker/卡片全部 `#[serde(default)]` 宽松解析，读路径静默降级（stale/损坏 → None，envs 哲学）。

## 模块（每文件 <400 行）

- `io.rs`：`atomic_write`（temp 兄弟文件+fsync+rename+父目录 fsync，unix 0o600，失败清 temp）/ `atomic_write_json` / `now_rfc3339`。
- `write.rs`：`save_resource_version(cat, name, [VersionFile]) -> u32`（`.tmp-v{n}.<pid>` 目录写全量后 rename 原子落位，dest 存在即败）；`create_agent`/`update_agent_refs`（逐字段 diff，仅变更字段追加 history 条目）/`delete_agent`（幂等；marker 由调用方先清）。
- `rollback.rs`：`rollback_resource`（版本必须 ∈ history 且目录存在；指针切换 + updated_at）。
- `references.rs`：`scan_resource`（prompts→soul|how|output 桑；skills→*.md 桑与含 SKILL.md 子目录；tools→直接子项；memory→memory.md）+ `references_snapshot`/`refresh_agent_references`（引用卡快照重算回写）。
- `nfs.rs`：nfsserve 0.11 `FileSystem` 只读实现——filehandle=规范相对路径字节（根编码为 `"/"`，拒绝 `..`/绝对/非 UTF8），getattr/read/readdir 真实透传，一切变更操作 `NFS3ERR_ROFS`。真实内核 mount 已验证。
- `serve.rs`：`NfsServerOpts/NfsServerStatus/NfsServerHandle` + `spawn_nfs_server`（独立 OS 线程跑 accept loop，watch-channel 优雅 shutdown，port 0=临时端口）+ `default_opts_from_config`（`agent.nfs{enabled,host,port=2049,read_only}`）。挂载：`mount -t nfs -o vers=3,tcp,port=P,mountport=P,nolock <host>:/ <dir>`（含 mount 协议，无需外部 mountd）。

## 接缝（谁消费什么）

core `resolve_agent`：builtin 优先 → 引用卡 → prompt 资源 current 版本 → `compose_prompt`（# Soul/# How/# Output + 追加 # Memory）；`effective_default_agent`：cli > active marker > `agent.default` > act。
- `tools_paths(scope, agent)` 已收口在 core `agent::resource`（Active=会话 agent 引用目录 / All=全部 tools 池），session 不为读侧帮助函数依赖本 crate。
- skill 遮蔽：`core::skill::discover()` roots = active agent 技能根在前 + 全局 skills 目录，first-wins（详见 [agents/core](../core/index.md)）。
- session：`/agent <名>` 切换（[agents/session](../session/index.md)）、bash 前缀 PATH 注入；web：`/api/agents*` CRUD + 激活 fan_out + NFS 生命周期（[agents/web](../web/index.md)）。
- `crates/agent`（opencoder-agent 舰队 worker 二进制）是**另一个 crate**，勿混淆。
