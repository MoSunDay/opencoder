Commit: 502c8352120f2d335200b8f96e55b90a471baf7e

# 二进制资源中的目录引用检查

目录清理原先将所有非文本文件视为无法核验，正常数据库或编译缓存会阻止无关的 Agent、DAG 和二进制资源删除。现在支持 SQLite 和 CPython 3.11 缓存，保持未知格式拒绝删除的规则。

SQLite 复制到临时目录后只读检查表、视图等结构与记录；UTF-16 文本由数据库引擎解码。WAL 校验头部、页大小、帧身份、校验和及最终提交，派生索引校验结构和校验和。原数据库、WAL 和索引不写入；首次读取与变化复核均限制在 4 MiB，文件增长也不会取消上限；不完整、损坏或变化中的文件阻止删除。缓存通过有长度、深度与对象数量限制的解析器提取字符串，不加载或执行代码。加密或无法解码的二进制值、虚拟表及其他缓存版本仍需核验，继续拒绝删除。

| 行为 | 定向测试 |
| --- | --- |
| UTF-8/UTF-16 数据库引用与源文件保持 | `sqlite_resource_references_are_read_from_a_copy_and_source_is_preserved` |
| 缓存中的依赖与无关常量 | `python_cache_constants_block_references_without_blocking_unrelated_cards` |
| 未知版本、截断、非法标志及尾随数据 | `invalid_or_unknown_python_cache_still_refuses_deletion` |
| 未知数据库值与损坏文件 | `unverified_database_values_and_corruption_still_refuse_deletion` |
| WAL 最新提交与源文件保持 | `active_database_wal_is_not_silently_omitted_or_modified` |
| DAG、二进制引用采用同一检查 | `sqlite_resources_protect_dag_and_binary_dependencies_too` |
| 不执行视图仍核对其定义 | `sqlite_view_definitions_are_checked_without_executing_the_view` |
| 不完整 WAL、错误校验和和非法索引 | `incomplete_or_corrupt_wal_and_unverified_index_cannot_allow_deletion` |
| 嵌套缓存、数字、Unicode 及每个截断位置 | `nested_python_code_constants_and_numeric_values_are_inspected_without_execution` |

定向目录保护集成测试 32 项通过，缓存解析单测 1 项通过，相关 Clippy 零警告；未运行全量测试。发布与稳定观察单独验收。

相关说明：[Control](../../../agents/control/index.md)、[能力库](../../brain/index.md)。
