import {Alert,Button,Form,Input,Spin} from 'antd';
import {useEffect,useMemo,useRef,useState} from 'react';
import {apiGet,apiPost} from './api.js';
import {err} from './notice.js';
import {useMessage} from './ui/appMessage.js';
import {FileWorkspace} from './ui/files/workspace.jsx';
import {directoryPaths} from './ui/files/format.js';
import {decodeFiles,EXAMPLE_SPEC,specFiles} from './todo/directory/model.js';
import {mergeBuiltinPrimaryAgents} from './agents/builtins.js';
import {changeEntry,directoryProblems} from './todo/directory/operations.js';
import {EntryDialog} from './todo/directory/operationDialog.jsx';
import {errorProblems,FileProblems} from './todo/directory/problems.jsx';

function TodoEditorSession({templateName,version,creating=false,onNotice,onClose,onCreated,onDirtyChange}) {
  const msg=useMessage();
  const [files,setFiles]=useState({});const [original,setOriginal]=useState({});
  const [selected,setSelected]=useState('objective.md');const [location,setLocation]=useState(null);
  const [loading,setLoading]=useState(true);const [saving,setSaving]=useState(false);const [loadError,setLoadError]=useState('');
  const [revision,setRevision]=useState('');const [activeVersion,setActiveVersion]=useState(version);
  const [name,setName]=useState('');const [agents,setAgents]=useState(null);
  const [problems,setProblems]=useState([]);const [serverProblems,setServerProblems]=useState([]);
  const [operation,setOperation]=useState(null);const [directories,setDirectories]=useState([]);
  const mounted=useRef(true);const savingRef=useRef(false);
  const changed=useMemo(()=>Array.from(new Set([...Object.keys(files),...Object.keys(original)])).filter(path=>files[path]!==original[path]),[files,original]);
  const dirty=changed.length>0 || directoryPaths(files,directories).join('\n')!==directoryPaths(original).join('\n') || creating && !!name;
  const decoded=useMemo(()=>decodeFiles(files,agents),[files,agents]);
  const localProblems=[...decoded.diagnostics,...directoryProblems(files,directories)];
  const diagnostics=[...localProblems,...serverProblems];
  useEffect(()=>{onDirtyChange?.(dirty);},[dirty,onDirtyChange]);
  useEffect(()=>{
    mounted.current=true;
    (async()=>{
      try {
        const [bundle,agentData]=await Promise.all([
          creating ? Promise.resolve({files:specFiles(EXAMPLE_SPEC),revision:''}) : apiGet(`/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/files`),
          apiGet('/api/agents'),
        ]);
        if (!mounted.current) return;
        if (!bundle?.files || typeof bundle.files!=='object') throw new Error('模板文件响应缺少 files');
        // /api/agents 只返回注册卡；内置 primary 三角色（act/plan/command，
        // 见 agents/builtins.js 与 core::builtin_agents）由本层并回可选集，
        // 否则模板里的 agent:'act'（如 EXAMPLE_SPEC）会被误判不可用。
        const allowed=mergeBuiltinPrimaryAgents((agentData.agents||[]).filter(a=>a.primary && a.name!=='workflow').map(a=>a.name));
        setFiles(bundle.files);setOriginal(bundle.files);setRevision(bundle.revision);setAgents(allowed);
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
    if(localProblems.length){setProblems(localProblems);return false;}
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
  const applyEntry=(operation,name)=>{
    const next=changeEntry({files,directories,selected},operation,name);
    setFiles(next.files);setDirectories(next.directories);setSelected(next.selected);setServerProblems([]);setLocation(null);
  };
  // Inline drafts (create/rename) arrive with a name; delete still confirms in a dialog.
  const handleOperation=operation=>{
    if (!operation.name) {setOperation(operation);return;}
    try {applyEntry(operation,operation.name);}
    catch(error){setProblems([{path:operation.path || 'workflow.json',message:error.message,line:1,column:1}]);}
  };
  return <section className="todo-directory-editor" aria-label="TODO 目录编辑器">
    <div className="todo-directory-toolbar">
      <Button onClick={onClose} disabled={saving}>返回</Button>
      <Button type="primary" loading={saving} disabled={loading||!!loadError} onClick={save}>保存</Button>
    </div>
    {loading ? <Spin/> : <>
    {loadError&&<Alert type="error" title="加载模板失败" description={loadError}/>}
    {creating&&<Form layout="vertical"><Form.Item label="模板名" required><Input aria-label="模板名" value={name} onChange={e=>setName(e.target.value)} disabled={saving}/></Form.Item></Form>}
    <FileWorkspace files={files} directories={directories} selected={selected} onSelect={setSelected} onOperation={handleOperation} diagnostics={diagnostics} changed={changed}
      readOnly={saving||!!loadError} onChange={change} onSave={save} onError={setProblems} location={location}/>
    <FileProblems problems={problems} onClose={()=>setProblems([])} onLocate={locate}/>
    {operation&&<EntryDialog operation={operation} onApply={name=>applyEntry(operation,name)} onClose={()=>setOperation(null)}/>}
    </>}
  </section>;
}

export function TodoEditor(props){return <TodoEditorSession key={`${props.templateName}/${props.version}`} {...props}/>;}
