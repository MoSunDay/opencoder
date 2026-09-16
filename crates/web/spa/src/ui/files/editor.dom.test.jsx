// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {beforeEach,expect,it,vi} from 'vitest';
import {act,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {EditorView} from '@codemirror/view';
import {undo} from '@codemirror/commands';
import {FileEditor} from './editor.jsx';
import {FileWorkspace} from './workspace.jsx';
beforeEach(()=>{Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});});
const cm=()=>EditorView.findFromDOM(document.querySelector('.cm-editor'));
it('each file keeps its own undo history and selection',()=>{
  const sessions=new Map();const changed=vi.fn();const view=render(<FileEditor path="a.md" value="first" sessions={sessions} onChange={changed}/>);
  act(()=>cm().dispatch({changes:{from:0,to:5,insert:'changed'},selection:{anchor:4}}));
  view.rerender(<FileEditor path="b.json" value="{}" sessions={sessions} onChange={changed}/>);
  view.rerender(<FileEditor path="a.md" value="changed" sessions={sessions} onChange={changed}/>);
  expect(cm().state.selection.main.head).toBe(4);act(()=>undo(cm()));expect(changed).toHaveBeenLastCalledWith('a.md','first');
});
it('Ctrl-S invokes save while read-only mode disallows editing and saving',()=>{
  const save=vi.fn();const changed=vi.fn();const view=render(<FileEditor path="task.json" value="{}" onChange={changed} onSave={save}/>);
  fireEvent.keyDown(screen.getByLabelText('文件内容 task.json'),{key:'s',code:'KeyS',ctrlKey:true});expect(save).toHaveBeenCalledOnce();
  view.rerender(<FileEditor path="task.json" value="{}" readOnly onChange={changed} onSave={save}/>);
  expect(cm().state.readOnly).toBe(true);expect(cm().contentDOM.getAttribute('contenteditable')).toBe('false');
  fireEvent.keyDown(screen.getByLabelText('文件内容 task.json'),{key:'s',code:'KeyS',ctrlKey:true});expect(save).toHaveBeenCalledOnce();
});
it('restoring a cached file binds callbacks to the current editor and ignores external updates',()=>{
  const sessions=new Map();const first=vi.fn();const current=vi.fn();
  const previous=render(<FileEditor path="task.json" value="{}" sessions={sessions} onChange={first}/>);
  previous.unmount();
  const next=render(<FileEditor path="task.json" value="{}" sessions={sessions} onChange={current}/>);
  act(()=>cm().dispatch({changes:{from:0,to:2,insert:'{"title":"new"}'}}));
  expect(current).toHaveBeenCalledWith('task.json','{"title":"new"}');expect(first).not.toHaveBeenCalled();
  current.mockClear();next.rerender(<FileEditor path="task.json" value={'{"title":"server"}'} sessions={sessions} onChange={current}/>);
  expect(cm().state.doc.toString()).toBe('{"title":"server"}');expect(current).not.toHaveBeenCalled();
});
it('read-only workspaces keep file selection and hide mutation menus',async()=>{
  const select=vi.fn();const operation=vi.fn();
  render(<FileWorkspace files={{'tasks/context.md':'context'}} selected="tasks/context.md" onSelect={select} onOperation={operation} readOnly/>);
  await waitFor(()=>expect(document.querySelector('[data-file-path="tasks/context.md"]')).toBeTruthy());
  const node=document.querySelector('[data-file-path="tasks/context.md"]');
  fireEvent.contextMenu(node);
  expect(document.querySelector('.ant-dropdown')).toBeNull();expect(operation).not.toHaveBeenCalled();
  fireEvent.click(node);expect(select).toHaveBeenCalledWith('tasks/context.md');
});
