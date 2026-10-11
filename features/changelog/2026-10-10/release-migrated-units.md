# 发布收尾跳过已不存在的历史服务

迁移后的发布记录仍可引用新机器上不存在的 systemd unit。控制器先读取 LoadState，只跳过 not-found；现存 unit 的停用失败仍正常上报，已经改变链接时仍执行一次 daemon-reload。修复避免实际切换完成后因历史服务缺失使信号发布回执失败。

`scripts/platform/rolling_tests/test_deployment.py` 的 25 项测试通过，覆盖缺失/现存服务混合、部分停用失败及原运行时不被停止。控制器按内容摘要独立安装，修复不改写既有发布包。
