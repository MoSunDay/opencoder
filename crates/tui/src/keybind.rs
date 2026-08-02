pub const HELP: &str = "\
快捷键列表：

  Shift+Tab        切换模式 act <--> plan（plan→act 清空上下文；Alt+Tab 同效）
  Enter            提交（空闲） / 转向（运行中，下一轮生效）
  Tab              提交（空闲） / 排队跟进（运行中，完成后提交）
  Ctrl+Shift+Tab  切换模式（保留上下文，不重置）
  Ctrl+T          仅切换状态，保留上下文（当 Ctrl+Shift+Tab 被拦截时使用）
  Ctrl+V          粘贴剪贴板图片（截图）
  Alt+回车            插入换行（多行输入）
  $                选择并插入技能 -> {$name}；提交时加载
  /                命令选择: /task（会话）, /config（设置）, /model（模型）, /compact（压缩）
  Shift+I          编辑计划（plan 模式、空闲时）: i/a 编辑, :wq 保存, :q! 放弃
  Esc              关闭帮助/弹窗/清空输入
  Esc Esc          双击 Esc 中断运行中的任务
  Ctrl+C          中断运行中的任务（同 Esc Esc）
  Ctrl+D           退出
  Ctrl+H           打开/关闭此帮助
  Ctrl+W           删除光标前的单词
  Ctrl+U           清空整个输入行（可被 Ctrl+Z 撤销）
  Ctrl+A / Ctrl+E  光标移到行首/行尾
  Ctrl+Z / Ctrl+Y  撤销 / 重做输入编辑
  ↑ / ↓            多行时移动光标；单行时浏览历史记录
  PageUp/Down      滚动对话记录  （PageDown = 跳到底部）
  Shift+PageUp/Down 滚动转向面板（查看更早的排队条目 / 回到最新）
  Ctrl+F           强制重新渲染屏幕
  Ctrl+L           退出子代理视图 / 折叠所有输出 / 回到底部跟随 / 清空输入
  Ctrl+L / Ctrl+U  /config、/model 弹窗内: 清空当前聚焦字段

鼠标:            滚轮滚动对话记录；点击箭头跟随最新
                  拖拽选择文本并复制到剪贴板（OSC52）
                  SHIFT+拖拽 = 选中并复制到剪贴板
                  转向面板: ✕ 删除, > 立即提交（中断并提升）
";
