Commit: b98058ed96d82224f4070da893aecb653fafc6c8

# plan question 输入框、光标与展示序号修复

## Context

question 弹窗的自定义输入原本只是一行带笔图标的文本，没有明确输入框；光标纵坐标又依赖问题和选项行数估算，长文本、窄终端和 Unicode 换行时会与实际渲染偏离。

## Change Summary

- 自定义输入改为独立三行边框框，固定在弹窗底部，上方内容换行不再改变光标位置。
- 光标以输入框内部矩形、Unicode 显示宽度和单行可见窗口计算，并为硬件光标保留最右侧单元。
- 预设选项仅在界面显示连续序号；Custom 不编号，`QuestionResponse` 仍保存和回填原始选项值。
- 预设答案追加补充说明、Custom 作为完整自定义答案的现有语义不变。

## Validation

- `cargo test -p opencoder-tui`
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings`
- 定向覆盖序号与原始答案隔离、独立输入框、CJK 光标、长输入和窄终端换行。

## Compatibility

- question tool schema、`QuestionHub`、批量确认顺序和 context 答案格式不变。
- 逻辑边界说明见 [tui 模块](../../../agents/tui/index.md)。
