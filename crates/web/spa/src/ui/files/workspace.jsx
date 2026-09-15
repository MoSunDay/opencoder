import {useMemo, useRef, useState} from 'react';
import {Empty, Input, Tree} from 'antd';
import {fileTree} from './format.js';
import {FileEditor} from './editor.jsx';

export function FileWorkspace({files, selected, onSelect, diagnostics = [], changed = [], children, ...editorProps}) {
  const [search,setSearch] = useState(''); const sessions = useRef(new Map());
  const paths = Object.keys(files).sort().filter(path => path.toLowerCase().includes(search.toLowerCase()));
  const tree = useMemo(() => fileTree(paths,diagnostics,changed),[paths.join('\n'),diagnostics,changed]);
  return <div className="file-workspace">
    <aside className="file-workspace-tree"><Input.Search aria-label="搜索文件" placeholder="搜索文件" value={search} onChange={e => setSearch(e.target.value)}/>
      {children}
      <Tree.DirectoryTree key={search} treeData={tree} defaultExpandAll selectedKeys={selected ? [selected] : []}
        onSelect={keys => {if (Object.hasOwn(files,keys[0])) onSelect(keys[0]);}} blockNode/>
    </aside>
    <main className="file-workspace-content">{Object.hasOwn(files,selected)
      ? <FileEditor path={selected} value={files[selected]} sessions={sessions.current} diagnostics={diagnostics} {...editorProps}/>
      : <Empty description="选择文件查看内容"/>}</main>
  </div>;
}
