# Native Harness DAG scheduling and evidence access

Dynamic steps support `failure_policy=collect_all`, and `trigger_rule=all_done` allows finalizers to run after failed or cancelled dependencies. Defaults preserve existing scheduling behavior. Cancellation gives configured native operations bounded cleanup time before killing their descendants.

The artifact browser supports dynamic instance indices and explicit artifact paths, including authenticated report archive downloads. Policy fields survive registration and are editable in the DAG UI.

Validation: workspace Rust regression completed with the dynamic timeout test rerun after enlarging its outer CI watchdog; all 10 dynamic tests pass without changing the tested one-second execution deadline. Workspace Clippy with warnings denied passes. SPA: 891 tests pass and production build succeeds. Platform release tests: 47 rolling and 12 signal tests pass. Native pool and business acceptance remain separately gated; these checks do not establish Native business success.
