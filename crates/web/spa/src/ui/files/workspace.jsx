import {useEffect,useMemo, useRef, useState} from 'react';
import {Empty, Input} from 'antd';
import {directoryPaths, fileTree} from './format.js';
import {FileEditor} from './editor.jsx';
import {FileTree} from './tree.jsx';

export function FileWorkspace({files, directories = [], selected, onSelect, onOperation, operationsReadOnly, allowCreateDirectory, diagnostics = [], changed = [], children, ...editorProps}) {
  const [search,setSearch] = useState(''); const sessions = useRef(new Map());
  const paths = Object.keys(files).sort().filter(path => path.toLowerCase().includes(search.toLowerCase()));
  const [expanded,setExpanded]=useState([]);
  const folders=directoryPaths(Object.fromEntries(paths.map(path=>[path,''])),directories.filter(path=>path.toLowerCase().includes(search.toLowerCase())));
  const folderKey=[...new Set(folders)].join('\n');
  useEffect(()=>{setExpanded(previous=>[...new Set([...previous,...folders])]);},[folderKey]);
  useEffect(()=>{if(selected)setExpanded(previous=>[...new Set([...previous,...directoryPaths({[selected]:''})])]);},[selected]);
  const tree = useMemo(() => fileTree(paths,diagnostics,changed,folders),[paths.join('\n'),folderKey,diagnostics,changed]);
  return <div className="file-workspace">
    <FileTree allowCreateDirectory={allowCreateDirectory} tree={tree} selected={selected} expanded={expanded} onExpand={setExpanded} onSelect={onSelect}
      onOperation={(operationsReadOnly ?? editorProps.readOnly) || !onOperation ? undefined : operation=>{setSearch('');onOperation(operation);}}>
      <Input.Search aria-label="搜索文件" placeholder="搜索文件" value={search} onChange={e => setSearch(e.target.value)}/>
      {children}
    </FileTree>
    <main className="file-workspace-content">{Object.hasOwn(files,selected)
      ? <FileEditor path={selected} value={files[selected]} sessions={sessions.current} diagnostics={diagnostics} {...editorProps}/>
      : <Empty description="选择文件查看内容"/>}</main>
  </div>;
}
