// @vitest-environment jsdom
import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import './test/setup-dom.js';
const api = vi.hoisted(() => ({apiGet:vi.fn(),apiPut:vi.fn(),apiPost:vi.fn(),apiPatch:vi.fn()}));
vi.mock('./api.js',() => api);
vi.mock('./ui/files/editor.jsx',() => ({FileEditor:({path,value,onChange,readOnly}) => <textarea aria-label={`文件内容 ${path}`} value={value} disabled={readOnly} onChange={e=>onChange(path,e.target.value)}/>}));
import {AgentDetail} from './agentDetail.jsx';
import {b64EncodeText} from './agentsItems.js';
const file = (path,text,mode=0o600) => ({path,content_b64:b64EncodeText(text),mode});
const snapshot = (cat,files=[],extra={}) => ({ok:true,category:cat,baseline:{resource:files.length ? 'shared' : null,version:files.length ? 2 : 0,revision:'revision'},versions:files.length ? [1,2] : [],files,read_only:false,...extra});
let views;
const button = name => screen.getByRole('button',{name:label=>label.replace(/\s/g,'')===name});
const tab = name => fireEvent.click(screen.getByRole('tab',{name,exact:true}));
const mount = async props => {
  const rendered = render(<AgentDetail name="coder" onNotice={vi.fn()} onChanged={vi.fn()} {...props}/>);
  await screen.findByLabelText('prompt-soul'); return rendered;
};
beforeEach(() => {
  views = {prompts:snapshot('prompts',[file('soul.md','SOUL'),file('how.md','HOW'),file('output.md','OUTPUT')]),
    skills:snapshot('skills',[file('probe/SKILL.md','SKILL'),file('probe/assets/data.bin','\0binary')]),
    tools:snapshot('tools',[file('run.sh','#!/bin/sh\necho yes',0o755)]),memory:snapshot('memory',[file('memory.md','MEMORY')])};
  api.apiGet.mockReset().mockImplementation(async path => {
    if (path.endsWith('/meta')) return {meta:{name:'coder',current:{prompt:'shared'},history:[{field:'prompt',from:null,to:'shared'}],references:{}}};
    const cat = path.split('/').at(-1);
    if (views[cat]) return views[cat];
    throw new Error(`unexpected GET ${path}`);
  });
  api.apiPut.mockReset().mockImplementation(async (path,body) => {
    const cat = path.split('/').at(-1); const original = views[cat];
    const files = original.files.filter(f=>!body.removed.includes(f.path) && !body.files.some(c=>c.path===f.path)).concat(body.files);
    views[cat] = {...original,files,baseline:{resource:'private',version:3,revision:'new'},versions:[1,2,3]}; return views[cat];
  });
  api.apiPost.mockReset().mockImplementation(async () => ({...views.prompts,files:[file('soul.md','RESTORED')],baseline:{resource:'private',version:3,revision:'restored'}}));
  api.apiPatch.mockReset().mockResolvedValue({ok:true});
  vi.spyOn(window,'confirm').mockReturnValue(false);
});
afterEach(() => {cleanup();vi.restoreAllMocks();});

describe('AgentDetail direct resources',() => {
  it('shows all four categories directly without pool or version selectors',async () => {
    await mount();
    expect(screen.getByLabelText('prompt-soul').value).toBe('SOUL');
    expect(screen.queryByLabelText('ref-select-prompt')).toBeNull();
    expect(screen.queryByLabelText('history-prompts')).toBeNull();
    tab('Skills'); expect(await screen.findByLabelText('文件内容 probe/SKILL.md')).toBeTruthy();
    tab('Tools'); expect((await screen.findByLabelText('文件内容 run.sh')).value).toContain('echo yes');
    tab('Memory'); expect((await screen.findByLabelText('文件内容 memory.md')).value).toBe('MEMORY');
    tab('Meta'); expect(await screen.findByText('引用变更历史')).toBeTruthy();
    expect(screen.getByText('— → shared')).toBeTruthy();
  });
  it('saves only changed files with the read baseline through agent identity',async () => {
    await mount(); const baseline=views.prompts.baseline;
    fireEvent.change(screen.getByLabelText('prompt-soul'),{target:{value:'EDITED'}}); fireEvent.click(button('保存'));
    await waitFor(()=>expect(api.apiPut).toHaveBeenCalledWith('/api/agents/coder/resources/prompts',{baseline,files:[file('soul.md','EDITED')],removed:[]}));
    expect(await screen.findByText('已保存')).toBeTruthy();
    expect(screen.getByLabelText('prompt-how').value).toBe('HOW');
  });
  it('retains drafts across tabs, callback rerenders and rejected refreshes',async () => {
    const view=await mount(); fireEvent.change(screen.getByLabelText('prompt-soul'),{target:{value:'DRAFT'}});
    tab('Memory'); fireEvent.change(await screen.findByLabelText('文件内容 memory.md'),{target:{value:'MEMORY DRAFT'}});
    tab('Prompt'); view.rerender(<AgentDetail name="coder" onNotice={vi.fn()} onChanged={vi.fn()}/>);
    expect(screen.getByLabelText('prompt-soul').value).toBe('DRAFT');
    fireEvent.click(button('刷新')); expect(window.confirm).toHaveBeenCalled(); expect(screen.getByLabelText('prompt-soul').value).toBe('DRAFT');
    const event=new Event('beforeunload',{cancelable:true}); window.dispatchEvent(event); expect(event.defaultPrevented).toBe(true);
    tab('Memory'); expect(screen.getByLabelText('文件内容 memory.md').value).toBe('MEMORY DRAFT');
  });
  it('allows first save of empty resources and requires a nonempty prompt',async () => {
    views.prompts=snapshot('prompts'); views.memory=snapshot('memory'); await mount();
    expect(button('保存').disabled).toBe(true);
    fireEvent.change(screen.getByLabelText('prompt-how'),{target:{value:'FIRST'}}); fireEvent.click(button('保存'));
    await waitFor(()=>expect(api.apiPut.mock.calls[0][1].baseline.resource).toBeNull());
    tab('Memory'); await screen.findByText('未配置，可新增文件或上传内容后保存。');
    fireEvent.click(button('新增文件'));
    fireEvent.change(screen.getByLabelText('文件路径'),{target:{value:'memory.md'}}); fireEvent.click(button('确认'));
    fireEvent.change(await screen.findByLabelText('文件内容 memory.md'),{target:{value:'NEW MEMORY'}}); fireEvent.click(button('保存'));
    await waitFor(()=>expect(api.apiPut.mock.calls.at(-1)[0]).toBe('/api/agents/coder/resources/memory'));
  });
  it('displays read errors and never enables overwrite',async () => {
    api.apiGet.mockImplementation(async path=>{if(path.endsWith('/meta'))return {meta:{name:'coder'}};throw new Error('read denied');});
    render(<AgentDetail name="coder"/>); expect(await screen.findByText('读取 Prompt 失败：read denied')).toBeTruthy();
    expect(screen.queryByLabelText('prompt-soul')).toBeNull(); expect(api.apiPut).not.toHaveBeenCalled();
  });
  it('keeps inputs when saving fails with a stale baseline',async () => {
    api.apiPut.mockRejectedValue(Object.assign(new Error('409 resource changed'),{status:409})); await mount();
    fireEvent.change(screen.getByLabelText('prompt-soul'),{target:{value:'KEEP ME'}}); fireEvent.click(button('保存'));
    expect(await screen.findByText('保存失败：409 resource changed')).toBeTruthy(); expect(screen.getByLabelText('prompt-soul').value).toBe('KEEP ME');
  });
  it('restores history as a new version and immediately refreshes content',async () => {
    await mount(); fireEvent.click(screen.getByText('历史版本'));
    fireEvent.mouseDown(screen.getByLabelText('history-prompts'));
    const option=await waitFor(()=>{const option=document.querySelector('.ant-select-item-option[title="v1"]');expect(option).toBeTruthy();return option;});
    fireEvent.click(option); fireEvent.click(button('恢复为新版本'));
    await waitFor(()=>expect(api.apiPost).toHaveBeenCalledWith('/api/agents/coder/resources/prompts/restore',{baseline:views.prompts.baseline,version:1}));
    await waitFor(()=>expect(screen.getByLabelText('prompt-soul').value).toBe('RESTORED'));
    expect(screen.getByLabelText('prompt-how').value).toBe('');
  });
  it('shows real builtin prompt and tools as read-only',async () => {
    views.prompts=snapshot('prompts',[],{read_only:true,builtin_prompt:'ACTUAL BUILTIN'});
    views.tools={...views.tools,read_only:true,tool_filter:{Allow:['bash','task']}}; views.skills=snapshot('skills',[],{read_only:true});
    render(<AgentDetail name="act"/>); expect(await screen.findByText('ACTUAL BUILTIN')).toBeTruthy(); expect(screen.queryByLabelText('prompt-soul')).toBeNull();
    tab('Tools'); expect(await screen.findByText('工具限制：bash、task')).toBeTruthy();
    expect(screen.getByLabelText('文件内容 run.sh').disabled).toBe(true); expect(screen.queryByRole('button',{name:'上传替换'})).toBeNull();
    tab('Skills'); expect(await screen.findByText('未配置。')).toBeTruthy(); expect(screen.queryByText(/可新增文件/)).toBeNull();
  });
  it('renames a skill directory with binary attachments and persists the file diff',async () => {
    await mount(); tab('Skills'); await screen.findByLabelText('文件内容 probe/SKILL.md');
    fireEvent.contextMenu(document.querySelector('[data-file-path="probe"]'));
    fireEvent.click(await screen.findByText('重命名目录'));
    const inline=await screen.findByLabelText('重命名目录名称'); fireEvent.change(inline,{target:{value:'renamed'}});
    fireEvent.keyDown(inline,{key:'Enter',code:'Enter',keyCode:13});
    fireEvent.click(button('保存')); await waitFor(()=>expect(api.apiPut).toHaveBeenCalled());
    const body=api.apiPut.mock.calls[0][1]; expect(body.removed).toEqual(['probe/SKILL.md','probe/assets/data.bin']); expect(body.files[0].path).toBe('renamed/SKILL.md');
    expect(body.files[1]).toEqual(file('renamed/assets/data.bin','\0binary'));
  });
  it('uploads binary replacements without changing executable permissions',async () => {
    await mount(); tab('Tools'); await screen.findByLabelText('文件内容 run.sh');
    fireEvent.change(screen.getByLabelText('上传替换'),{target:{files:[new File([new Uint8Array([0,255,2])],'new.bin')]}});
    await screen.findByText(/二进制 · 权限 755/); fireEvent.click(button('保存'));
    await waitFor(()=>expect(api.apiPut).toHaveBeenCalled());
    expect(api.apiPut.mock.calls[0][1].files).toEqual([{path:'run.sh',content_b64:'AP8C',mode:0o755}]);
  });
  it('adds and removes files and reopens saved contents',async () => {
    views.tools=snapshot('tools'); const view=await mount(); tab('Tools'); fireEvent.click(button('新增文件'));
    fireEvent.change(screen.getByLabelText('文件路径'),{target:{value:'new.sh'}}); fireEvent.click(button('确认'));
    fireEvent.change(await screen.findByLabelText('文件内容 new.sh'),{target:{value:'echo new'}}); fireEvent.click(button('保存'));
    await screen.findByText('已保存'); view.unmount(); await mount(); tab('Tools'); expect((await screen.findByLabelText('文件内容 new.sh')).value).toBe('echo new');
    fireEvent.click(button('移除')); fireEvent.click(button('确认')); fireEvent.click(button('保存'));
    await waitFor(()=>expect(api.apiPut.mock.calls.at(-1)[1].removed).toEqual(['new.sh']));
  });
  // 全局激活已移除：详情不再提供「设为生效」（会话级 agent 切换走会话接口）。
});
