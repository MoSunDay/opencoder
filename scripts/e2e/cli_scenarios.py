"""CLI-reachable e2e scenarios against real glm5.2.

Contract depth map (HARD = deterministic store/contract assert;
SOFT = model-cooperation-dependent, recorded as skip not failure):
  E7/E9   config -> models display                 HARD
  E1/E6   write+bash produces a real artifact      HARD (file) + SOFT (tool markers)
  E2      --continue LOADS prior context           HARD (history has turn-1 content)
  E5      --fork copies messages + leaves original untouched   HARD (role-seq + count)
  E3      compaction fires + summary is content-aware          SOFT (volume-dependent)
  E3b     post-compaction --continue retains usable context    SOFT
  E4      subagent dispatched + completed + tracked in DB      SOFT (dispatch) + HARD (if ran)
  E8      bundle export/import roundtrip integrity             HARD (count + content equal)
  E10     plan agent cannot mutate disk                       HARD (no file created)
  E12     session list + delete lifecycle                     HARD (store CRUD via CLI)
  E13     interleaved thinking reasoning_content persisted    SOFT (model-dependent)
  E14     config show emits valid merged JSON                 HARD (deterministic)
"""

from __future__ import annotations

import json
import os
import subprocess

from . import lib
from .lib import Counter

# Distinctive prompts (kept identical to the prior bash suite for continuity).
SNAKE_PROMPT = (
    "用 python3 写一个终端贪吃蛇游戏，保存为 snake.py。方向键控制、吃食物变长、"
    "撞墙/撞自己结束。写完运行 'python3 -m py_compile snake.py' 验证语法（不要运行游戏循环）。"
)
RESUME_PROMPT = (
    "给贪吃蛇加计分板，屏幕顶部显示当前分数和最高分(high score)。修改 snake.py。"
)
FORK_PROMPT = "给贪吃蛇加暂停功能，按 P 键暂停/继续。"
THUNDER_PROMPT = (
    "用 python3 写一个雷霆战机(飞机射击)游戏，保存为 thunder.py。方向键移动、空格射击、"
    "敌机下落、击中得分、被撞结束。写完运行 'python3 -m py_compile thunder.py' 验证语法（不要运行游戏循环）。"
)


def _py_compile(path: str) -> bool:
    return subprocess.run(["python3", "-m", "py_compile", path]).returncode == 0


def _line_count(path: str) -> int:
    try:
        with open(path) as f:
            return sum(1 for _ in f)
    except FileNotFoundError:
        return 0


def _has_tool_use(log: str) -> bool:
    return lib.TOOL_START in log


def _assert_game_artifact(c: Counter, sid: str | None, log: str, path: str, label: str) -> None:
    """Shared E1/E6 contract: the model wrote a substantial, compiling program
    via tools. If the live run itself errored (no session marker — transient
    model/network failure), soft-skip the dependent checks rather than emit a
    spurious contract failure. Only HARD-fail when the run succeeded yet the
    artifact is missing (a real write-toolchain regression)."""
    if sid is None:
        c.soft(f"{label}: run completed", False, "no [session] marker (run errored — transient)")
        return
    c.check(f"{label} exists", os.path.isfile(path))
    lc = _line_count(path)
    c.check(f"{label} > 50 lines ({lc})", lc > 50)
    c.check(f"{label} compiles", os.path.isfile(path) and _py_compile(path))
    c.soft("tools were actually invoked (write/bash)", _has_tool_use(log), "no ▸ marker in log")


def run_all(bin_path: str, api_key: str) -> Counter:
    c = Counter()
    base_cfg = lib.make_config(api_key=api_key)  # threshold 100k -> compaction off
    compact_cfg = lib.make_config(
        api_key=api_key,
        reasoning_effort="low",
        max_tokens=8192,
        context_threshold=2000,
        tail_turns=1,
        reserved=2000,
    )
    plan_cfg = lib.make_config(api_key=api_key, reasoning_effort="low", max_tokens=8192)

    snake = lib.seed_workdir(base_cfg)
    thunder = lib.seed_workdir(base_cfg)
    compact = lib.seed_workdir(compact_cfg)
    probe = lib.seed_workdir(base_cfg)
    plan = lib.seed_workdir(plan_cfg)
    lib.write_file(probe, "hello.py", 'def greet(name):\n    print(f"Hello, {name}!")\n\ngreet("World")\n')

    # ---- E7/E9: cheap display-path checks (no model call) ----
    print("== E7/E9: reasoning_effort + interleaved_thinking config -> models display ==")
    models = lib.models_text(bin_path, snake)
    c.check("models shows thinking line", "thinking" in models and ":" in models)
    c.check("reasoning_effort resolved to medium", "medium" in models)
    c.check("models shows interleave line", "interleave" in models and ":" in models)
    c.check("interleaved_thinking defaults on", "on" in models)

    # ---- E1: write+bash a real snake game ----
    print("== E1: glm writes a runnable snake game (write+bash tools) ==")
    rc, e1_log = lib.run_prompt(bin_path, snake, SNAKE_PROMPT)
    snake_py = os.path.join(snake, "snake.py")
    sid = lib.extract_session_id(e1_log)
    _assert_game_artifact(c, sid, e1_log, snake_py, "snake.py")
    # sid is consumed by E2/E5/E8; capture whether the run produced a usable session.
    c.check("captured SNAKE session id", sid is not None)

    # ---- E2: --continue resumes SAME session AND loads prior context ----
    print("== E2: --continue resumes same session and loads prior context ==")
    rc, e2_log = lib.run_prompt(bin_path, snake, RESUME_PROMPT, "--continue")
    rsid = lib.extract_session_id(e2_log)
    # The live run must complete before we can assert resume contracts. A missing
    # session marker means run_headless errored (transient model/network failure);
    # skip the dependent checks rather than emit spurious contract failures.
    if rsid is None:
        c.soft("E2 --continue run completed", False,
               "no [session] marker in log (run errored — transient)")
    else:
        c.check("resume reuses same session id", sid == rsid, f"{sid} vs {rsid}")
        sjson = lib.show_json(bin_path, snake, sid)
        roles = lib.message_roles(sjson)
        # Deep: resumed history must contain the turn-1 user prompt (context loaded,
        # not a fresh session). The user prompt text is the strongest evidence.
        c.check("resumed history contains turn-1 prompt (context loaded)",
                "贪吃蛇" in lib.all_text(sjson))
        c.check("resume appended a turn (>= 2 user messages)",
                roles.count("user") >= 2, f"roles={roles}")
        # scoreboard added to the artifact
        sc = 0
        try:
            with open(snake_py, encoding="utf-8") as f:
                sc = sum(1 for ln in f if any(k in ln.lower() for k in ("high", "score")))
        except FileNotFoundError:
            pass
        c.soft("scoreboard added to snake.py", sc > 0, f"{sc} score-related lines")

    # ---- E5: --fork copies messages AND leaves original untouched ----
    print("== E5: --fork copies messages and leaves original untouched ==")
    if sid:
        before = lib.show_json(bin_path, snake, sid)
        before_roles = lib.message_roles(before)
        before_count = len(before["messages"])
        rc, e5_log = lib.run_prompt(bin_path, snake, FORK_PROMPT, "--session", sid, "--fork")
        fsid = lib.extract_fork_id(e5_log)
        c.check("fork creates a new session id", fsid is not None and fsid != sid,
                f"orig={sid} fork={fsid}")
        after = lib.show_json(bin_path, snake, sid)
        c.check("original session message count unchanged by fork",
                len(after["messages"]) == before_count,
                f"{before_count} -> {len(after['messages'])}")
        c.check("original session role sequence unchanged by fork",
                lib.message_roles(after) == before_roles)
        if fsid:
            fjson = lib.show_json(bin_path, snake, fsid)
            f_roles = lib.message_roles(fjson)
            c.check("fork copied the original message history",
                    len(fjson["messages"]) >= before_count,
                    f"fork={len(fjson['messages'])} orig={before_count}")
            c.check("fork history prefix matches original",
                    f_roles[: len(before_roles)] == before_roles)
            c.check("fork diverged (added its own turn)",
                    len(fjson["messages"]) > before_count)

    # ---- E3: compaction fires + produces a content-aware summary ----
    print("== E3: compaction auto-triggers with low context_threshold ==")
    compact_logs = []
    prompts = [
        "用 Python 写一个模块 calc.py，包含 add, subtract, multiply, divide, power, sqrt 六个函数，"
        "每个函数都要有完整的类型注解、docstring 和错误处理。写完运行 'python3 -m py_compile calc.py' 验证语法。",
        "给 calc.py 加 log, sin, cos, tan 四个函数，同样需要类型注解和 docstring。修改 calc.py。",
        "给 calc.py 加一个 main 函数，从命令行参数读取表达式并计算结果。",
    ]
    csid = None
    for i, p in enumerate(prompts):
        flags = [] if i == 0 else ["--continue"]
        rc, log = lib.run_prompt(bin_path, compact, p, *flags)
        compact_logs.append(log)
        if i == 0:
            csid = lib.extract_session_id(log)
    combined = "\n".join(compact_logs)
    compacted = lib.COMPACTION_MARK in combined
    c.soft("compaction triggered (only real compactions emit the marker)", compacted)
    if compacted:
        # Deep: the summary must reference the actual work, not be generic boilerplate.
        idx = combined.find(lib.COMPACTION_MARK)
        snippet = combined[idx : idx + 400]
        c.check("compaction summary is non-trivial", len(snippet) > len(lib.COMPACTION_MARK) + 20)
        c.soft("compaction summary references the work (calc/functions)",
               any(k in snippet for k in ("calc", "函数", "add", "subtract", "math")),
               "summary did not mention prior work")

    # ---- E3b: post-compaction --continue retains usable context ----
    print("== E3b: --continue works after compaction (context retained) ==")
    rc, e3b_log = lib.run_prompt(
        bin_path, compact,
        "给 calc.py 的每个函数加一个单元测试，保存为 test_calc.py，用 pytest 风格。测试要 import calc 并调用实际的函数。",
        "--continue",
    )
    resumed_csid = lib.extract_session_id(e3b_log)
    c.soft("session resumed after compaction", resumed_csid is not None)
    test_calc = os.path.join(compact, "test_calc.py")
    if os.path.isfile(test_calc):
        with open(test_calc, encoding="utf-8") as f:
            tc = f.read()
        # Deep business contract: post-compaction the model still knew calc's API
        # (proves the summary + tail turns preserved usable context).
        c.soft("post-compaction test references calc's functions",
               "import calc" in tc or "calc." in tc,
               "test_calc.py did not import/use calc")
    else:
        c.soft("post-compaction produced test_calc.py", False, "file not created")

    # ---- E4: subagent (task tool) dispatched + completed + tracked ----
    print("== E4: subagent (task tool) dispatch, completion, and DB tracking ==")
    rc, e4_log = lib.run_prompt(
        bin_path, probe,
        "请使用 task 工具（subagent_type 设为 explore）派遣一个子代理来分析 hello.py 的代码结构，"
        "然后基于子代理返回的结果给我一个中文总结。不要自己直接读取文件，必须通过 task 工具完成调查。",
    )
    pid = lib.extract_session_id(e4_log)
    dispatched = lib.SUBAGENT_START in e4_log
    c.soft("subagent dispatched via task tool", dispatched, "model may not use task tool")
    if dispatched and pid:
        # Hard DB contract: a subagent task row exists and reached a terminal status.
        pjson = lib.show_json(bin_path, probe, pid)
        tasks = pjson.get("subagent_tasks", [])
        c.check("subagent_tasks row persisted", len(tasks) >= 1)
        if tasks:
            t = tasks[-1]
            c.check("subagent reached terminal status",
                    t.get("status") in ("completed", "failed"))
            c.soft("subagent result is non-empty",
                   bool(str(t.get("result") or "").strip()))
        # Did the parent answer actually cite the investigation?
        c.soft("parent answer references the subagent finding",
               "hello" in lib.assistant_text(pjson).lower() or "greet" in lib.assistant_text(pjson).lower(),
               "parent text did not mention hello.py")

    # ---- E6: cross-game-type regression (write+bash) ----
    print("== E6: glm writes a runnable thunder-fighter game (cross-game regression) ==")
    rc, e6_log = lib.run_prompt(bin_path, thunder, THUNDER_PROMPT)
    thunder_py = os.path.join(thunder, "thunder.py")
    _assert_game_artifact(c, lib.extract_session_id(e6_log), e6_log, thunder_py, "thunder.py")

    # ---- E8: bundle export/import roundtrip INTEGRITY ----
    print("== E8: bundle export/import roundtrip integrity ==")
    if sid:
        bundle = os.path.join(tempfile_dir(), "snake.opencoder")
        rc, _ = lib.run(bin_path, ["--workdir", snake, "session", "export", sid, "--out", bundle])
        c.check("bundle exported", os.path.isfile(bundle))
        rc, imp_out = lib.run(bin_path, ["--workdir", snake, "session", "import", bundle])
        c.check("bundle imported", "imported session" in imp_out)
        # Extract the imported id from the import output / list.
        isid = _extract_imported_id(imp_out)
        if isid:
            orig = lib.show_json(bin_path, snake, sid)
            imp = lib.show_json(bin_path, snake, isid)
            oc, ic = len(orig["messages"]), len(imp["messages"])
            c.check("roundtrip message count equal", oc == ic, f"orig={oc} imported={ic}")
            # Deep: content integrity — every original text block survives the roundtrip.
            c.check("roundtrip text content identical",
                    lib.all_text(orig) == lib.all_text(imp))
            c.check("roundtrip role sequence identical",
                    lib.message_roles(orig) == lib.message_roles(imp))

    # ---- E10: plan agent is read-only (cannot mutate disk) ----
    print("== E10: plan agent cannot create files (read-only contract) ==")
    target = os.path.join(plan, "plan_test.py")
    if os.path.exists(target):
        os.remove(target)
    rc, e10_log = lib.run_prompt(
        bin_path, plan,
        "创建一个新文件 plan_test.py，内容为：print('created by plan agent')。直接用 bash 写入文件。",
        "--agent", "plan",
    )
    # Hard business contract: regardless of HOW the plan agent responds (tries bash
    # and is blocked, or just describes a plan), it must NOT have created the file.
    c.check("plan agent created no file (disk unmutated)", not os.path.isfile(target))

    # ---- E12: session list + delete lifecycle ----
    print("== E12: session list + delete lifecycle ==")
    e12_dir = lib.seed_workdir(base_cfg)
    # Empty workdir: no sessions yet.
    rc, empty_list = lib.run(bin_path, ["--workdir", e12_dir, "session", "list"])
    c.check("empty workdir shows no sessions", "no sessions" in empty_list.lower())
    # Create a session (lightweight prompt — no tool use needed).
    rc, e12_log = lib.run_prompt(bin_path, e12_dir, "回复一句话：你好世界。")
    sid12 = lib.extract_session_id(e12_log)
    if sid12:
        # List: the created session must appear.
        list_out = lib.session_list(bin_path, e12_dir)
        c.check("session list shows created session", sid12 in list_out)
        # Delete it.
        del_out = lib.session_delete(bin_path, e12_dir, sid12)
        c.check("session delete succeeds", "deleted" in del_out.lower())
        # List again: the deleted session must be gone.
        list_after = lib.session_list(bin_path, e12_dir)
        c.check("deleted session no longer listed", sid12 not in list_after)
    else:
        c.soft("E12 run completed", False, "no [session] marker (run errored — transient)")

    # ---- E13: interleaved thinking reasoning_content persisted (soft) ----
    print("== E13: interleaved thinking reasoning_content persisted (soft) ==")
    # Reuse the E1 snake session — a tool-using turn should persist Reasoning
    # blocks when interleaved_thinking is on (default) and the model emits
    # reasoning_content.  Model-dependent: skip (not fail) when absent.
    if sid:
        sjson = lib.show_json(bin_path, snake, sid)
        c.soft("reasoning_content persisted as Reasoning block (interleaved thinking)",
               lib.has_reasoning_blocks(sjson),
               "model may not emit reasoning_content on this turn")
    else:
        c.soft("E13 skipped (no E1 session)", False, "E1 did not produce a session")

    # ---- E14: config show emits valid merged JSON ----
    print("== E14: config show emits valid merged JSON ==")
    cfg_out = lib.config_show(bin_path, snake)
    cfg_valid = False
    cfg_fields = False
    try:
        cfg_json = json.loads(cfg_out)
        cfg_valid = True
        cfg_fields = all(k in cfg_json for k in ("model", "provider", "compaction"))
    except Exception:
        pass
    c.check("config show is valid JSON", cfg_valid)
    c.check("config JSON has core fields (model/provider/compaction)", cfg_fields)

    c.summary("CLI scenarios")
    return c


def tempfile_dir() -> str:
    import tempfile
    return tempfile.gettempdir()


def _extract_imported_id(imp_out: str) -> str | None:
    # The import command prints: "imported session <ID> (...)" then
    # "continue with: opencoder --session <ID>"
    import re
    m = re.search(r"imported session ([0-9A-Z]+)", imp_out)
    return m.group(1) if m else None
