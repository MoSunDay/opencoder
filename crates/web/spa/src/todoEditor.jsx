import {Alert,Button,Form,Input,Modal,Select,Space,Spin,Tag,Typography} from 'antd';
import {useEffect,useMemo,useRef,useState} from 'react';
import {apiGet,apiPost} from './api.js';
import {err} from './notice.js';
import {useMessage} from './ui/appMessage.js';
import {FileWorkspace} from './ui/files/workspace.jsx';
import {changeTask,decodeFiles,EXAMPLE_SPEC,setProperty,specFiles} from './todo/directory/model.js';
import {errorProblems,FileProblems} from './todo/directory/problems.jsx';

function TodoEditorSession({templateName,version,creating=false,onNotice,onClose,onCreated,onDirtyChange}) {
  const msg=useMessage();
  const [files,setFiles]=useState({});const [original,setOriginal]=useState({});
  const [selected,setSelected]=useState('objective.md');const [location,setLocation]=useState(null);
  const [loading,setLoading]=useState(true);const [saving,setSaving]=useState(false);const [loadError,setLoadError]=useState('');
  const [revision,setRevision]=useState('');const [activeVersion,setActiveVersion]=useState(version);
  const [name,setName]=useState('');const [agents,setAgents]=useState(null);const [envs,setEnvs]=useState([]);
  const [problems,setProblems]=useState([]);const [serverProblems,setServerProblems]=useState([]);
  const [operation,setOperation]=useState(null);const [taskId,setTaskId]=useState('');
  const mounted=useRef(true);const savingRef=useRef(false);
  const changed=useMemo(()=>Array.from(new Set([...Object.keys(files),...Object.keys(original)])).filter(path=>files[path]!==original[path]),[files,original]);
  const dirty=changed.length>0 || creating && !!name;
  const decoded=useMemo(()=>decodeFiles(files,agents),[files,agents]);
  const diagnostics=[...decoded.diagnostics,...serverProblems];
  useEffect(()=>{onDirtyChange?.(dirty);},[dirty,onDirtyChange]);
  useEffect(()=>{
    mounted.current=true;
    (async()=>{
      try {
        const [bundle,agentData,envData]=await Promise.all([
          creating ? Promise.resolve({files:specFiles(EXAMPLE_SPEC),revision:''}) : apiGet(`/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/files`),
          apiGet('/api/agents'),apiGet('/api/todo/envs'),
        ]);
        if (!mounted.current) return;
        if (!bundle?.files || typeof bundle.files!=='object') throw new Error('模板文件响应缺少 files');
        const allowed=(agentData.agents||[]).filter(a=>a.primary && a.name!=='workflow').map(a=>a.name);
        setFiles(bundle.files);setOriginal(bundle.files);setRevision(bundle.revision);setAgents(allowed);setEnvs(envData.envs||[]);
        const errors=[...(bundle.diagnostics||[]),...decodeFiles(bundle.files,allowed).diagnostics];
        if(errors.length)setProblems(errors);
      }catch(error){if(mounted.current){setLoadError(error.message);setProblems(errorProblems(error));onNotice?.(err('加载模板失败: '+error.message));}}
      finally{if(mounted.current)setLoading(false);}
    })();
    return()=>{mounted.current=false;};
  },[templateName,version,creating]);
  const change=(path,text)=>{setFiles(previous=>({...previous,[path]:text}));setServerProblems([]);};
  const report=error=>{
    const failures=errorProblems(error);setProblems(failures);setServerProblems(error.body?.diagnostics||[]);
  };
  const check=async()=>{
    if(decoded.diagnostics.length){setProblems(decoded.diagnostics);return false;}
    try{await apiPost('/api/todo/validate-files',{files});setServerProblems([]);return true;}
    catch(error){report(error);return false;}
  };
  const save=async()=>{
    if(savingRef.current || loading || loadError)return;
    if(creating && !name.trim()){setProblems([{path:'todo.json',message:'请输入模板名'}]);return;}
    savingRef.current=true;setSaving(true);
    try{
      if(!await check())return;
      if(creating){
        await apiPost('/api/todo/templates',{name:name.trim(),files});
        setOriginal(files);onDirtyChange?.(false);onCreated?.();return;
      }
      const result=await apiPost(`/api/todo/templates/${encodeURIComponent(templateName)}/new-version`,{
        source_version:activeVersion,expected_revision:revision,files,
      });
      setActiveVersion(result.version);setRevision(result.revision);setOriginal(files);setServerProblems([]);
      msg.success(`已保存为 ${result.version}，并设为当前版本`);
    }catch(error){report(error);}
    finally{savingRef.current=false;if(mounted.current)setSaving(false);}
  };
  const locate=problem=>{
    if(/^(workflow\.json|env\.json|objective\.md|todos\/[^/]+\/(task\.json|context\.md|instructions\.md|acceptance\.md))$/.test(problem.path)) {
      setFiles(previous=>Object.hasOwn(previous,problem.path)?previous:{...previous,[problem.path]:''});
    }
    setSelected(problem.path);setLocation({...problem,key:Date.now()});
  };
  const selectedTodo=/^todos\/([^/]+)(?:\/|$)/.exec(selected)?.[1];
  const act=()=>{
    try{
      const next=changeTask(files,operation,selectedTodo,taskId.trim());setFiles(next);setServerProblems([]);
      setSelected(operation==='delete'?'workflow.json':`todos/${taskId.trim()}/task.json`);setOperation(null);
    }catch(error){setProblems(errorProblems(error,selected));}
  };
  if(loading)return <Spin/>;
  return <section className="todo-directory-editor" aria-label="TODO 目录编辑器">
    {loadError&&<Alert type="error" title="加载模板失败" description={loadError}/>}
    {creating&&<Form layout="vertical"><Form.Item label="模板名" required><Input aria-label="模板名" value={name} onChange={e=>setName(e.target.value)} disabled={saving}/></Form.Item></Form>}
    <div className="todo-parent-heading"><strong>父 Agent · workflow</strong><span>负责调度与验收，每个 TODO 独立执行</span></div>
    <div className="todo-directory-toolbar"><Space wrap>
      {!creating&&<Tag>{templateName} / {activeVersion}</Tag>}
      <Button disabled={saving||!!loadError} onClick={()=>{setTaskId('');setOperation('add');}}>新增 TODO</Button>
      <Button disabled={!selectedTodo||saving} onClick={()=>{setTaskId(`${selectedTodo}-copy`);setOperation('copy');}}>复制 TODO</Button>
      <Button disabled={!selectedTodo||saving} onClick={()=>{setTaskId(selectedTodo);setOperation('rename');}}>重命名 TODO</Button>
      <Button disabled={!Object.hasOwn(files,selected)||saving} onClick={()=>{
        setFiles(previous=>Object.fromEntries(Object.entries(previous).filter(([path])=>path!==selected)));setServerProblems([]);
      }}>移除文件</Button>
      <Button danger disabled={!selectedTodo||saving} onClick={()=>setOperation('delete')}>删除 TODO</Button>
    </Space><Space wrap>
      <Typography.Text type={diagnostics.length?'danger':'secondary'}>{diagnostics.length?`${diagnostics.length} 个错误`:dirty?'有未保存修改':'文件已同步'}</Typography.Text>
      <Button disabled={saving||!!loadError} onClick={async()=>{if(await check())msg.success('全部文件符合 TODO 框架要求');}}>校验文件</Button>
      <Button onClick={onClose} disabled={saving}>返回</Button>
      <Button type="primary" loading={saving} disabled={!!loadError} onClick={save}>{creating?'创建模板':'保存新版本'}</Button>
    </Space></div>
    <Space wrap style={{marginBottom:12}}><Typography.Text>执行 Agent：{agents?.join('、') || '无可用 Primary Agent'}</Typography.Text>
      <Select aria-label="模板环境" placeholder="选择环境写入 env.json" style={{minWidth:200}} disabled={saving||!!loadError}
        options={[{value:'',label:'不绑定环境'},...envs.map(e=>({value:e.name,label:e.name}))]}
        onChange={env=>{try{change('env.json',setProperty(files['env.json'],'env',env||null));setSelected('env.json');}catch(error){report(error);}}}/>
    </Space>
    <FileWorkspace files={files} selected={selected} onSelect={setSelected} diagnostics={diagnostics} changed={changed}
      readOnly={saving||!!loadError} onChange={change} onSave={save} onError={setProblems} location={location}/>
    <FileProblems problems={problems} onClose={()=>setProblems([])} onLocate={locate}/>
    <Modal title={({add:'新增 TODO',copy:'复制 TODO',rename:'重命名 TODO',delete:'删除 TODO'})[operation]} open={!!operation}
      onCancel={()=>setOperation(null)} onOk={act} okText="确定" cancelText="取消">
      {operation==='delete'?<p>删除 {selectedTodo} 的任务目录；存在依赖引用时需要先修改引用。</p>:<Input aria-label="TODO ID" value={taskId} onChange={e=>setTaskId(e.target.value)} onPressEnter={act}/>}
    </Modal>
  </section>;
}

export function TodoEditor(props){return <TodoEditorSession key={`${props.templateName}/${props.version}`} {...props}/>;}
