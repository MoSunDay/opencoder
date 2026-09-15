import {describe,it,expect} from 'vitest';
import {EXAMPLE_SPEC,specFiles,decodeFiles,changeTask,setProperty} from './model.js';
import {formatJson} from '../../ui/files/format.js';

describe('TODO directory contract',()=>{
  it('round trips complete task context, metadata, and acceptance requirements',()=>{
    const spec=structuredClone(EXAMPLE_SPEC);spec.todos[0].acceptance.required_tool_calls=[{name:'bash',arguments_contains:{command:'check'},result_ok:false}];
    const {spec:decoded,diagnostics}=decodeFiles(specFiles(spec),['act']);
    expect(diagnostics).toEqual([]);expect(decoded.todos[0].acceptance).toEqual(spec.todos[0].acceptance);
    expect(decoded.objective).toBe(spec.objective);expect(decoded.metadata).toEqual(spec.metadata);
  });
  it('reports invalid JSON and missing Markdown with exact file paths',()=>{
    const files=specFiles(EXAMPLE_SPEC);files['env.json']='{\n "env": }';delete files['todos/t1/context.md'];
    const problems=decodeFiles(files).diagnostics;
    expect(problems.some(p=>p.path==='env.json'&&p.line===2)).toBe(true);
    expect(problems.some(p=>p.path==='todos/t1/context.md')).toBe(true);
  });
  it('rejects unknown files, fields, unavailable agents and invalid dependency graphs',()=>{
    const files=specFiles(EXAMPLE_SPEC);files['extra.md']='lost data';
    expect(decodeFiles(files).diagnostics.some(p=>p.path==='extra.md')).toBe(true);
    delete files['extra.md'];files['todos/t1/task.json']=setProperty(files['todos/t1/task.json'],'depends_on',['t1']);
    expect(decodeFiles(files).diagnostics.some(p=>p.message.includes('自身'))).toBe(true);
    expect(decodeFiles(specFiles(EXAMPLE_SPEC),[]).diagnostics.some(p=>p.message.includes('Agent'))).toBe(true);
  });
  it('renames dependencies while retaining raw metadata numbers and task contents',()=>{
    let files=changeTask(specFiles(EXAMPLE_SPEC),'copy','t1','t2');
    files['todos/t2/task.json']=setProperty(files['todos/t2/task.json'],'depends_on',['t1']);
    files['workflow.json']=files['workflow.json'].replace('"metadata": {}','"metadata": {"number":9007199254740993}');
    files=changeTask(files,'rename','t1','renamed');
    expect(files['workflow.json']).toContain('9007199254740993');
    expect(JSON.parse(files['todos/t2/task.json']).depends_on).toEqual(['renamed']);
    expect(files['todos/renamed/context.md']).toBe(EXAMPLE_SPEC.todos[0].requirement_background);
    expect(()=>changeTask(files,'delete','renamed')).toThrow('依赖');
  });
  it('formats JSON without rounding numbers or changing string values',()=>{
    const text='{"number":9007199254740993,"s":"a\\n\\\"b","items":[{},true,null,1e100]}';
    expect(formatJson(text)).toContain('9007199254740993');
    expect(formatJson(text)).toContain('1e100');
    expect(JSON.parse(formatJson(text))).toEqual(JSON.parse(text));
    expect(()=>formatJson('{')).toThrow();
  });
});
