"""Runs inside the real DAG container; computes evidence without a model."""
import ast
import hashlib
import json
import os
from pathlib import Path
import sys
import time


def expression(text):
    tree = ast.parse(text, mode='eval')
    allowed = (ast.Expression, ast.BinOp, ast.Add, ast.Sub, ast.Mult, ast.UnaryOp,
               ast.USub, ast.UAdd, ast.Name, ast.Load, ast.Call, ast.Constant)
    for node in ast.walk(tree):
        if not isinstance(node, allowed):
            raise ValueError('only arithmetic expressions are supported')
        if isinstance(node, ast.Name) and node.id not in ('a', 'b', 'abs'):
            raise ValueError('unknown expression name')
        if isinstance(node, ast.Call) and (not isinstance(node.func, ast.Name)
                or node.func.id != 'abs' or len(node.args) != 1 or node.keywords):
            raise ValueError('only abs(value) calls are supported')
        if isinstance(node, ast.Constant) and not isinstance(node.value, (int, float)):
            raise ValueError('only numeric constants are supported')
    return compile(tree, '<acceptance arithmetic>', 'eval')


def calculate(mode, text):
    if mode == 'oversize':
        return {'summary': 'x' * 17000, 'passed': False, 'failures': ['late negative verdict']}
    if mode == 'missing':
        return {'summary': 'finished without required check'}
    if mode == 'error':
        raise RuntimeError('intentional native execution failure')
    program = expression(text)
    source = f'def calculate(a, b):\n    return {text}\n'
    revision = hashlib.sha256(source.encode()).hexdigest()
    result = {'summary': 'Execution finished successfully', 'source': source,
              'revision': revision, 'args': [text], 'mode': mode}
    if mode == 'prepare':
        Path('calculate.py').write_text(source)
        Path('artifacts.json').write_text(json.dumps({'files': [{
            'path': 'calculate.py', 'bytes': len(source.encode()), 'sha256': revision}]}))
        result['prepared'] = True
    else:
        cases = {'fast': [(2, 3)], 'hold': [(0, 0), (8, 2)],
                 'edge': [(-2, 3), (-4, -5), (0, -7)]}[mode]
        checks = [{'a': a, 'b': b, 'expected': a + b,
                   'actual': eval(program, {'__builtins__': {}, 'abs': abs}, {'a': a, 'b': b})}
                  for a, b in cases]
        result.update(checks=checks, passed=all(c['actual'] == c['expected'] for c in checks))
        result['failures'] = [c for c in checks if c['actual'] != c['expected']]
    return result


def main():
    mode = sys.argv[1]
    if mode in ('hold', 'hold-error'):
        deadline = time.monotonic() + 900
        Path('entered').write_text(str(os.getpid()))
        while not Path('release').exists():
            if time.monotonic() >= deadline:
                raise TimeoutError('acceptance did not release the gate')
            time.sleep(.1)
    result = calculate('error' if mode == 'hold-error' else mode,
                       sys.argv[2] if len(sys.argv) > 2 else 'abs(a) + abs(b)')
    Path('output.json').write_text(json.dumps(result))
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
