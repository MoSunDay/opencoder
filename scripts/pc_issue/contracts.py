"""Pure construction of pinned, idempotent device and build execution requests."""
import hashlib
import json
import re


def inspection(snapshot):
    execution = snapshot.get('execution', {})
    status = execution.get('status')
    if not isinstance(status, str):
        raise ValueError('Execution response has no status')
    return {'status': status,
            'terminal': status in ('done', 'error', 'cancelled', 'interrupted'),
            'error': snapshot.get('error'), 'details': snapshot}


def identity(root, stage, round_number, kind):
    if not re.fullmatch(r'brain-[A-Za-z0-9_.-]+', root) or stage not in ('reproduce', 'verify'):
        raise ValueError('Expected root brain ID and reproduce/verify stage')
    if round_number not in (1, 2):
        raise ValueError('At most two rounds')
    return f'{kind}-pc-{root[6:]}-{stage}-{round_number}'


def device_request(root, stage, round_number, node, source, case_ids):
    if not node or not case_ids or len(set(case_ids)) != len(case_ids):
        raise ValueError('Explicit node and unique original case IDs required')
    if not all(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,99}', c) for c in case_ids):
        raise ValueError('Invalid original case ID')
    return {'id': identity(root, stage, round_number, 'dag'), 'kind': 'dag',
            'target': 'device-cases', 'node_id': node,
            'input': {'device_count': 1, 'case_source': source, 'case_ids': case_ids}}


def build_request(root, round_number, node, source):
    required = ('repo', 'branch', 'commit', 'worktree', 'purpose', 'original_assertions', 'changes')
    if not node or any(not source.get(key) for key in required):
        raise ValueError('Build requires node, repo, branch, full commit, worktree, purpose, assertions and changes')
    if not re.fullmatch(r'[a-f0-9]{40}', source['commit']) or source['purpose'] not in ('fix', 'diagnostic'):
        raise ValueError('Build source must have full commit and explicit fix/diagnostic purpose')
    frozen = json.dumps(source, ensure_ascii=False, sort_keys=True)
    prompt = ('执行单：要求 turn1 即执行。只派 Windows 构建成员，不扩展平台。'
              '核对源代码 commit 和 changes 中全部 diff_sha256，将精确修改应用到独立构建工作区；'
              '禁止只构建 branch HEAD。缺少源码或修改时明确失败。'
              '构建 Windows 完整可安装包，并提供用于隔离部署的便携 ZIP 包（保留全部运行依赖），给出构建退出码、制品大小/SHA、独立下载 SHA、'
              '源码与修改清单及实际成员执行 ID。不要发布或合并产品。\n冻结输入：' + frozen)
    return {'id': identity(root, 'verify', round_number, 'team'), 'kind': 'team',
            'target': 'jy-builder', 'node_id': node, 'input': {'prompt': prompt,
            'source': source, 'source_sha256': hashlib.sha256(frozen.encode()).hexdigest()}}
