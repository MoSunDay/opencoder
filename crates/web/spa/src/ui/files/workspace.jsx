import {useEffect,useMemo, useRef, useState} from 'react';
import {Empty, Input, Tree} from 'antd';
import {fileTree} from './format.js';
import {FileEditor} from './editor.jsx';

export function FileWorkspace({files, selected, onSelect, diagnostics = [], changed = [], children, ...editorProps}) {
  const [search,setSearch] = useState(''); const sessions = useRef(new Map());
  const paths = Object.keys(files).sort().filter(path => path.toLowerCase().includes(search.toLowerCase()));
  const [expanded,setExpanded]=useState([]);
  const folders=paths.flatMap(path=>path.split('/').slice(0,-1).map((_,i)=>path.split('/').slice(0,i+1).join('/')));
  const folderKey=[...new Set(folders)].join('\n');
  useEffect(()=>{setExpanded(previous=>[...new Set([...previous,...folders])]);},[folderKey]);
  const tree = useMemo(() => fileTree(paths,diagnostics,changed),[paths.join('\n'),diagnostics,changed]);
  return <div className="file-workspace">
    <aside className="file-workspace-tree"><Input.Search aria-label="搜索文件" placeholder="搜索文件" value={search} onChange={e => setSearch(e.target.value)}/>
      {children}
      <Tree.DirectoryTree key={search} treeData={tree} expandedKeys={expanded} onExpand={setExpanded} selectedKeys={selected ? [selected] : []}
        titleRender={node=><span data-file-path={node.key} title={node.key}>{node.title}</span>}
        onSelect={keys => {if (Object.hasOwn(files,keys[0])) onSelect(keys[0]);}} blockNode/>
    </aside>
    <main className="file-workspace-content">{Object.hasOwn(files,selected)
      ? <FileEditor path={selected} value={files[selected]} sessions={sessions.current} diagnostics={diagnostics} {...editorProps}/>
      : <Empty description="选择文件查看内容"/>}</main>
  </div>;
}
