import {useEffect, useMemo, useRef, useState} from 'react';
import {Dropdown, Input, Tree} from 'antd';

const root = {path:'',isDirectory:true};
const DRAFT_KEY = '__inline-draft__';
const ACTIONS = ['create-file','create-directory','rename'];

const parentPath = path => path.split('/').slice(0,-1).join('/');
const ancestors = path => path.split('/').filter(Boolean).map((_, index, parts) => parts.slice(0, index + 1).join('/'));

// Every existing file/directory key, for inline duplicate checks.
const collectKeys = (nodes, into = new Set()) => {
  (nodes || []).forEach(node => {into.add(node.key); collectKeys(node.children, into);});
  return into;
};

// Inline draft validation. Create allows implicit nested paths (`a/b/c.md`,
// intermediate folders are filled in by the caller); rename is a single
// segment. Returns '' when the draft is acceptable, so an empty input stays
// neutral until the user types.
export function inlineDraftError({action, path, parent}, value, keys) {
  const name = value.trim();
  if (!name) return '';
  if (action === 'rename' && /[\\/]/.test(name)) return '名称不能包含 \\ 或 /';
  const parts = name.split('/').map(part => part.trim());
  if (parts.some(part => !part || part === '.' || part === '..' || /[\\\0]/.test(part)))
    return '名称不能包含 \\、.. 等非法字符';
  const targets = parts.map((_, index) => [...ancestors(parent), ...parts.slice(0, index + 1)].filter(Boolean).join('/'));
  const final = targets[targets.length - 1];
  const clash = action === 'rename' ? final !== path && keys.has(final) : targets.some(target => keys.has(target));
  return clash ? '同名文件或目录已存在' : '';
}

// VSCode-style inline row: autofocus, Enter commits, Esc/blur cancels.
function DraftInput({label, placeholder, initial, invalid, value, onChange, onSubmit, onCancel}) {
  const ref = useRef(null);
  useEffect(() => {
    ref.current?.focus({cursor: initial ? 'all' : 'end'});
  },[]);
  const stop = event => event.stopPropagation();
  return <Input ref={ref} size="small" aria-label={label} placeholder={placeholder} value={value}
    status={invalid ? 'error' : undefined} onClick={stop}
    onChange={event => onChange(event.target.value)}
    onKeyDown={event => {
      event.stopPropagation();
      if (event.key === 'Enter') onSubmit();
      else if (event.key === 'Escape') onCancel();
    }}
    onBlur={onCancel}/>;
}

export function FileTree({tree, selected, expanded, onExpand, onSelect, onOperation, allowCreateDirectory = true, children}) {
  const [target,setTarget] = useState(root);
  const [open,setOpen] = useState(false);
  const [draft,setDraft] = useState(null);
  const [value,setValue] = useState('');
  const keys = useMemo(() => collectKeys(tree), [tree]);
  const kind = target.isDirectory ? '目录' : '文件';
  const items = [
    {key:'create-file',label:'新增文件'},
    ...(allowCreateDirectory ? [{key:'create-directory',label:'新增目录'}] : []),
    ...(target.path ? [
      {type:'divider'},
      {key:'rename',label:`重命名${kind}`},
      {key:'delete',label:`删除${kind}`,danger:true},
    ] : []),
  ];
  const startDraft = operation => {
    const parent = operation.action === 'rename' ? parentPath(operation.path)
      : operation.isDirectory ? operation.path : parentPath(operation.path);
    setDraft({...operation, parent});
    setValue(operation.action === 'rename' ? operation.path.split('/').pop() : '');
    if (parent && onExpand) onExpand([...new Set([...(expanded || []), ...ancestors(parent), parent])]);
  };
  const cancelDraft = () => {setDraft(null);setValue('');};
  const submitDraft = () => {
    if (!draft || inlineDraftError(draft, value, keys)) return;
    const name = value.trim();
    setDraft(null);setValue('');
    onOperation({action: draft.action, path: draft.path, isDirectory: draft.isDirectory, name});
  };
  const draftKind = draft ? (draft.action === 'create-directory' || (draft.action === 'rename' && draft.isDirectory) ? '目录' : '文件') : '';
  const draftInput = draft && <DraftInput label={`${draft.action === 'rename' ? '重命名' : '新增'}${draftKind}名称`}
    placeholder={draft.action === 'create-directory' ? '目录路径，如 docs/guide' : draft.action === 'create-file' ? '文件路径，如 docs/guide.md' : undefined}
    initial={draft.action === 'rename' ? draft.path.split('/').pop() : ''}
    invalid={!!value.trim() && !!inlineDraftError(draft, value, keys)}
    value={value} onChange={setValue} onSubmit={submitDraft} onCancel={cancelDraft}/>;
  const treeData = useMemo(() => {
    if (!draft || draft.action === 'rename') return tree;
    const node = {key: DRAFT_KEY, title: draftInput, isLeaf: draft.action === 'create-file', selectable: false};
    if (!draft.parent) return [...tree, node];
    const insert = nodes => nodes.map(item => item.key === draft.parent
      ? {...item, children:[...(item.children || []), node]}
      : item.children ? {...item, children: insert(item.children)} : item);
    return insert(tree);
  },[tree, draft, draftInput]);
  const content = <aside className="file-workspace-tree" aria-label="文件目录"
    onContextMenu={onOperation ? event => {
      const node = event.target.closest('.ant-tree-treenode')?.querySelector('[data-file-path]');
      const next = node ? {path:node.dataset.filePath,isDirectory:node.dataset.fileKind === 'directory'} : root;
      setTarget(next);
      if (!next.isDirectory) onSelect(next.path);
    } : undefined}>
    {children}
    <Tree.DirectoryTree treeData={treeData} expandedKeys={expanded} onExpand={onExpand}
      selectedKeys={open && target.path ? [target.path] : selected ? [selected] : []}
      titleRender={node => {
        if (draft && draft.action === 'rename' && node.key === draft.path) return draftInput;
        if (node.key === DRAFT_KEY) return node.title;
        return <span data-file-path={node.key} data-file-kind={node.isLeaf ? 'file' : 'directory'} title={node.key}>{node.title}</span>;
      }}
      onSelect={(keys,info) => {if (info.node.isLeaf) onSelect(keys[0]);}} blockNode/>
    {draft && <div className="file-inline-error" role="alert">{inlineDraftError(draft, value, keys)}</div>}
  </aside>;
  return onOperation ? <Dropdown trigger={['contextMenu']} open={open} onOpenChange={setOpen}
    menu={{items,onClick:({key}) => {
      setOpen(false);
      if (ACTIONS.includes(key)) startDraft({...target, action:key});
      else onOperation({...target, action:key});
    }}}>{content}</Dropdown> : content;
}
