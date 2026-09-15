import {useEffect, useRef, useState} from 'react';
import {Button, Segmented, Space} from 'antd';
import {basicSetup} from 'codemirror';
import {EditorState, Compartment, StateEffect, Annotation} from '@codemirror/state';
import {EditorView, keymap} from '@codemirror/view';
import {indentWithTab} from '@codemirror/commands';
import {json} from '@codemirror/lang-json';
import {markdown} from '@codemirror/lang-markdown';
import {setDiagnostics} from '@codemirror/lint';
import {Markdown} from '../../project/markdown.jsx';
import {formatJson,jsonLocation} from './format.js';
import './files.css';

const externalChange = Annotation.define();
const theme = EditorView.theme({
  '&': {height:'100%',backgroundColor:'var(--oc-bg-container, transparent)',color:'var(--oc-text, inherit)'},
  '.cm-scroller': {overflow:'auto',fontFamily:'var(--oc-mono, monospace)',fontSize:'13px'},
  '.cm-content': {minHeight:'320px'},
  '.cm-gutters': {backgroundColor:'transparent',color:'var(--oc-text-secondary, #777)'},
});

export function FileEditor({path, value = '', readOnly = false, diagnostics = [], onChange, onSave, onError, location, sessions}) {
  const host = useRef(null); const view = useRef(null); const localSessions = useRef(new Map());
  const cache = sessions || localSessions.current;
  const refs = useRef({}); refs.current = {onChange, onSave, path, readOnly};
  const permission = useRef(new Compartment());
  const [mode, setMode] = useState('source');
  const isMarkdown = /\.md$/i.test(path || '');
  useEffect(() => {
    if (!path) return;
    let saved = cache.get(path);
    const language = isMarkdown ? markdown() : /\.json$/i.test(path) ? json() : [];
    const extensions=[basicSetup, language, theme, EditorView.lineWrapping,
      permission.current.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
      EditorView.contentAttributes.of({'aria-label':`文件内容 ${path}`}),
      keymap.of([{key:'Mod-s',run:() => {if (!refs.current.readOnly) refs.current.onSave?.(); return true;}},indentWithTab]),
      EditorView.updateListener.of(update => {
        if (update.docChanged && !update.transactions.some(transaction=>transaction.annotation(externalChange)))
          refs.current.onChange?.(refs.current.path, update.state.doc.toString());
      }),
    ];
    if (!saved || saved.state.doc.toString() !== value) {
      saved = {state:EditorState.create({doc:value,extensions}),scrollTop:0,scrollLeft:0};
    } else {
      saved = {...saved,state:saved.state.update({effects:StateEffect.reconfigure.of(extensions)}).state};
    }
    const editor = new EditorView({state:saved.state, parent:host.current}); view.current = editor;
    editor.scrollDOM.scrollTop = saved.scrollTop; editor.scrollDOM.scrollLeft = saved.scrollLeft;
    return () => {
      cache.set(path,{state:editor.state,scrollTop:editor.scrollDOM.scrollTop,scrollLeft:editor.scrollDOM.scrollLeft});
      editor.destroy(); view.current = null;
    };
  },[path,cache]); // Each file retains its own history and selection.
  useEffect(() => {
    const editor = view.current;
    if (editor && editor.state.doc.toString() !== value) editor.dispatch({changes:{from:0,to:editor.state.doc.length,insert:value},annotations:externalChange.of(true)});
  },[value,path]);
  useEffect(() => {
    view.current?.dispatch({effects:permission.current.reconfigure([EditorState.readOnly.of(readOnly),EditorView.editable.of(!readOnly)])});
  },[readOnly,path]);
  useEffect(() => {
    const editor = view.current; if (!editor) return;
    const items = diagnostics.filter(d => d.path === path).map(d => {
      const line = editor.state.doc.line(Math.max(1,Math.min(d.line || 1,editor.state.doc.lines)));
      const from = Math.min(line.to,line.from + Math.max(0,(d.column || 1)-1));
      return {from,to:Math.min(editor.state.doc.length,from+1),severity:'error',message:d.message};
    });
    editor.dispatch(setDiagnostics(editor.state,items));
  },[diagnostics,path,value]);
  useEffect(() => {
    const editor = view.current; if (!editor || location?.path !== path) return;
    const line = editor.state.doc.line(Math.max(1,Math.min(location.line || 1,editor.state.doc.lines)));
    const anchor = Math.min(line.to,line.from + Math.max(0,(location.column || 1)-1));
    setMode('source'); editor.dispatch({selection:{anchor},scrollIntoView:true}); editor.focus();
  },[location,path]);
  const format = () => {
    try {
      const editor = view.current;
      if (editor) editor.dispatch({changes:{from:0,to:editor.state.doc.length,insert:formatJson(editor.state.doc.toString())}});
    } catch (error) { onError?.([{path,message:error.message,...jsonLocation(value)}]); }
  };
  const displayMode = isMarkdown ? mode : 'source';
  return <section className="file-editor" aria-label={`编辑器 ${path}`}>
    <div className="file-editor-toolbar"><strong>{path}</strong><Space wrap>
      {readOnly && <span>只读</span>}
      {!readOnly && path?.endsWith('.json') && <Button size="small" onClick={format}>格式化 JSON</Button>}
      {isMarkdown && <Segmented value={mode} onChange={setMode} options={[{value:'source',label:'源码'},{value:'preview',label:'预览'},{value:'split',label:'分屏'}]}/>}
    </Space></div>
    <div className={`file-editor-body file-editor-${displayMode}`}>
      <div ref={host} className="file-editor-code" style={{display:displayMode === 'preview' ? 'none' : undefined}}/>
      {isMarkdown && displayMode !== 'source' && <div className="file-editor-preview"><Markdown text={value}/></div>}
    </div>
  </section>;
}
