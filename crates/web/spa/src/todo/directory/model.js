import {validateSpec} from '../editor/specValidate.js';

export const EXAMPLE_SPEC = {schema_version:1,id:'wf-example',name:'示例工作流',objective:'完成任务并提供可核验结果',constraints:[],
  todos:[{id:'t1',title:'示例任务',requirement_background:'需要完成并验收当前任务',instructions:'完成任务并记录验证结果',depends_on:[],agent:'act',max_attempts:3,acceptance:{criteria:'结果满足目标并有验证依据'},metadata:{}}],metadata:{}};
export const pretty = value => JSON.stringify(value,null,2);
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const safeId = value => typeof value === 'string' && value.trim() && value !== '.' && !/[\\/\0]/.test(value) && !value.includes('..') && new TextEncoder().encode(value).length <= 128;

export function specFiles(spec, env = null) {
  const {objective,todos,...workflow} = spec;
  const files = {'workflow.json':pretty({...workflow,todos:todos.map(t => t.id)}),'objective.md':objective,'env.json':pretty({env})};
  for (const todo of todos) {
    const {id,requirement_background,instructions,acceptance,...task} = todo;
    files[`todos/${id}/task.json`] = pretty({...task,required_tool_calls:acceptance?.required_tool_calls || []});
    files[`todos/${id}/context.md`] = requirement_background;
    files[`todos/${id}/instructions.md`] = instructions;
    files[`todos/${id}/acceptance.md`] = acceptance?.criteria || '';
  }
  return files;
}

export function decodeFiles(files, agents) {
  const diagnostics = []; const parsed = {};
  const fail = (path,message,line=1,column=1) => diagnostics.push({path,message,line,column});
  for (const [path,text] of Object.entries(files)) {
    if (!path.endsWith('.json')) continue;
    try { parsed[path] = JSON.parse(text); }
    catch (error) {
      const position = /position (\d+)/.exec(error.message); const prefix = text.slice(0,position ? Number(position[1]) : 0);
      fail(path,error.message,prefix.split('\n').length,prefix.length-prefix.lastIndexOf('\n'));
    }
  }
  const required = (path, markdown=false) => {
    if (!Object.hasOwn(files,path)) fail(path,'缺少必需文件');
    else if (markdown && !files[path].trim()) fail(path,'必需的 Markdown 内容不能为空');
  };
  const keys = (path, allowed) => {
    const value = parsed[path];
    if (value === undefined) return;
    if (!object(value)) { fail(path,'文件内容必须是 JSON 对象'); return; }
    for (const key of Object.keys(value)) if (!allowed.includes(key)) fail(path,`不支持的字段：${key}`);
  };
  required('workflow.json'); required('env.json'); required('objective.md',true);
  keys('workflow.json',['schema_version','id','name','constraints','todos','metadata']);
  keys('env.json',['env']);
  const workflow = parsed['workflow.json']; const binding = parsed['env.json'];
  if (object(binding) && binding.env !== null && !safeId(binding.env)) fail('env.json','env 必须为合法环境名称或 null');
  if (object(workflow) && workflow.constraints !== undefined && (!Array.isArray(workflow.constraints) || workflow.constraints.some(c => typeof c !== 'string'))) fail('workflow.json','constraints 必须是字符串数组');
  const ids = Array.isArray(workflow?.todos) ? workflow.todos : [];
  if (!ids.length) fail('workflow.json','todos 必须是非空任务 ID 数组');
  const expected = new Set(['workflow.json','env.json','objective.md']); const todos = [];
  for (const id of ids) {
    if (!safeId(id)) { fail('workflow.json',`非法 TODO 目录名：${String(id)}`); continue; }
    const base = `todos/${id}`; const path = `${base}/task.json`;
    for (const file of ['task.json','context.md','instructions.md','acceptance.md']) {
      const name = `${base}/${file}`; expected.add(name); required(name,file.endsWith('.md'));
    }
    keys(path,['title','agent','depends_on','max_attempts','required_tool_calls','metadata']);
    const task = parsed[path]; if (!object(task)) continue;
    if (task.required_tool_calls?.some?.(call => call?.result_ok !== undefined && typeof call.result_ok !== 'boolean')) fail(path,'required_tool_calls.result_ok 必须是布尔值');
    if (agents && !agents.includes(task.agent)) fail(path,`不可用的 Primary Agent：${task.agent}`);
    todos.push({...task,id,depends_on:task.depends_on || [],requirement_background:files[`${base}/context.md`],instructions:files[`${base}/instructions.md`],
      acceptance:{criteria:files[`${base}/acceptance.md`],required_tool_calls:task.required_tool_calls || []}});
  }
  if (object(workflow)) for (const path of Object.keys(files)) if (!expected.has(path)) fail(path,'文件不属于 TODO 框架定义，或所属任务未列入 workflow.json');
  const spec = {...workflow,objective:files['objective.md'],todos};
  if (!diagnostics.length) for (const problem of validateSpec(spec)) {
    const match = /^todos\[(.*)\]$/.exec(problem.path);
    fail(match ? `todos/${match[1]}/task.json` : 'workflow.json',problem.message);
  }
  return {spec,diagnostics};
}

// Replace a root JSON property without reserializing unrelated metadata/numbers.
export function setProperty(text, key, value) {
  const source = JSON.parse(text); if (!object(source)) throw new Error('文件必须是 JSON 对象');
  const tokens = [...text.matchAll(/"(?:\\.|[^"\\])*"|[{}\[\],:]|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null/g)];
  let depth = 0;
  for (let i=0;i<tokens.length;i++) {
    const token=tokens[i][0];
    if (depth === 1 && token.startsWith('"') && tokens[i+1]?.[0] === ':' && JSON.parse(token) === key) {
      const start=i+2; let end=start; let nested=0;
      for (;end<tokens.length;end++) {
        const current=tokens[end][0];
        if (!nested && (current === ',' || current === '}')) break;
        if (current === '{' || current === '[') nested++;
        if (current === '}' || current === ']') nested--;
      }
      const from=tokens[start].index;const to=tokens[end-1].index+tokens[end-1][0].length;
      return text.slice(0,from)+pretty(value)+text.slice(to);
    }
    if (token === '{' || token === '[') depth++;
    if (token === '}' || token === ']') depth--;
  }
  const end=text.lastIndexOf('}');
  return text.slice(0,end)+(Object.keys(source).length ? ',' : '')+`\n${JSON.stringify(key)}: ${pretty(value)}\n`+text.slice(end);
}

export function changeTask(files, action, id, nextId) {
  const workflow=JSON.parse(files['workflow.json']); const ids=workflow.todos;
  if (!Array.isArray(ids)) throw new Error('workflow.json 的 todos 必须是数组');
  if (action !== 'add' && !ids.includes(id)) throw new Error('请选择已有 TODO');
  if (action !== 'delete' && (!safeId(nextId) || ids.includes(nextId))) throw new Error('TODO ID 非法或已存在');
  const result={...files};
  if (action === 'delete') {
    const consumers=ids.filter(other => other !== id && JSON.parse(files[`todos/${other}/task.json`]).depends_on?.includes(id));
    if (consumers.length) throw new Error(`以下任务仍依赖 ${id}：${consumers.join('、')}`);
  }
  if (action === 'add') {
    const starter=specFiles(EXAMPLE_SPEC);
    for (const [path,text] of Object.entries(starter)) if (path.startsWith('todos/t1/')) result[path.replace('todos/t1/',`todos/${nextId}/`)]=text;
  } else if (action === 'copy' || action === 'rename') {
    for (const [path,text] of Object.entries(files)) if (path.startsWith(`todos/${id}/`)) result[path.replace(`todos/${id}/`,`todos/${nextId}/`)]=text;
  }
  if (action === 'delete' || action === 'rename') for (const path of Object.keys(result)) if (path.startsWith(`todos/${id}/`)) delete result[path];
  const nextIds=action==='delete' ? ids.filter(item=>item!==id) : action==='rename' ? ids.map(item=>item===id?nextId:item) : [...ids,nextId];
  result['workflow.json']=setProperty(files['workflow.json'],'todos',nextIds);
  if (action === 'rename') for (const other of nextIds) {
    const path=`todos/${other}/task.json`;const task=JSON.parse(result[path]);
    if (task.depends_on?.includes(id)) result[path]=setProperty(result[path],'depends_on',task.depends_on.map(dep=>dep===id?nextId:dep));
  }
  return result;
}
