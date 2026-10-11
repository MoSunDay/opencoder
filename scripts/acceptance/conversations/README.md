# 远端会话验收

`browser.js` 验证真实 Web 的 Agent 与 Operator 会话：节点选择、身份核对、只读工具调用、续聊和重开。

```sh
node scripts/acceptance/conversations/browser.js \
  --base http://127.0.0.1:18081 \
  --token-file /private/server.token \
  --node NODE --agent AGENT \
  --output /absolute/new-acceptance-directory
```

执行节点需已就绪并具备所选身份的模型配置。脚本通过页面提交请求，核对持久化工具结果、受理耗时、浏览器错误及 HTTP 响应。输出目录须不存在；`result.json` 与截图保存验收结果。
