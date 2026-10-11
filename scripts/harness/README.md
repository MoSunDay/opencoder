# 宿主机 Codex 代理入口

`codex_proxy.py PROXY_RUNTIME CODEX [ARGS...]` 复用执行节点已安装的代理运行库，在每次执行时发现当前本地 SOCKS5 代理并建立随进程退出的 HTTP 桥接。参数与标准输入原样交给 Codex；不启动 TUI，不修改登录数据。

运行库目录必须包含一个以 `_socks5.py` 结尾的模块，导出 `running_proxy_url()`，以及提供 `run(arguments)` 的 `codex_proxy_exec.py`。模块及依赖由节点环境维护；缺少或存在多个代理模块时拒绝启动。通过现有 Harness `startup_script` 传入该目录和启动参数，使用 Python 3.11 或更高版本。不要把运行过程中产生的桥接端口或凭据存入 Server 配置；每次续聊同样需要重新建立桥接。

此入口用于普通宿主机会话。DAG 容器的依赖制备及网络路径仍遵循其独立执行约定。

定向回归：

```sh
python3 -m unittest discover -s scripts/harness -p 'test_codex_proxy.py' -v
```
