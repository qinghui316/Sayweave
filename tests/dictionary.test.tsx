import {automaticSettings} from './settings-fixtures';
import {act,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,expect,it,vi} from 'vitest';
import {Dictionary} from '../src/features/dictionary';
import {BrowserDictionary} from '../src/adapters/dictionary-browser';
import {DictionaryError,type DictionaryPage} from '../src/contracts/dictionary';
describe('manual dictionary',()=>{
  it('keeps legacy mappings stored while exposing only terms',async()=>{
    const port=new BrowserDictionary();
    const legacy=await port.create({kind:'correction',original:'飞格玛',replacement:'Figma',category:null});
    await port.create({kind:'term',original:'Sayweave',replacement:null,category:null});
    const list=vi.spyOn(port,'list');
    render(<Dictionary automaticTerms={automaticSettings()} port={port}/>);
    await screen.findByText('Sayweave');
    expect(screen.queryByText('飞格玛')).not.toBeInTheDocument();
    expect(screen.queryByRole('button',{name:'词典更多操作'})).not.toBeInTheDocument();
    expect(screen.getByRole('switch',{name:'自动入词'})).toBeVisible();
    expect(list.mock.calls.every(([query])=>query.kind==='term')).toBe(true);
    expect(await port.get('correction',legacy.id)).toEqual(legacy);
  });
  it('update Save/Cancel preserves the word and continues only after a confirmed save',async()=>{
    const port=new BrowserDictionary(),install=vi.fn();let guard:((action:()=>void)=>void)|null=null;
    const create=vi.spyOn(port,'create').mockRejectedValueOnce(new DictionaryError('busy'));
    render(<Dictionary automaticTerms={automaticSettings()} port={port} onGuardChange={value=>{guard=value;}}/>);
    const user=userEvent.setup();await user.click(await screen.findByRole('button',{name:'添加词语'}));await user.type(screen.getByLabelText('词语'),'绮蓝');
    act(()=>guard?.(install));await user.click(within(screen.getByRole('alertdialog')).getByRole('button',{name:'取消'}));expect(install).not.toHaveBeenCalled();expect(screen.getByLabelText('词语')).toHaveValue('绮蓝');
    act(()=>guard?.(install));await user.click(within(screen.getByRole('alertdialog')).getByRole('button',{name:'保存'}));expect(await screen.findByRole('alert')).toHaveTextContent('正在忙');expect(screen.getByLabelText('词语')).toHaveValue('绮蓝');expect(install).not.toHaveBeenCalled();
    act(()=>guard?.(install));await user.click(within(screen.getByRole('alertdialog')).getByRole('button',{name:'保存'}));await waitFor(()=>expect(install).toHaveBeenCalledOnce());expect((await port.list({kind:'term',search:'',page:1})).total).toBe(1);expect(create).toHaveBeenCalledTimes(2);
  });
  it('preserves input on failure, saves only after confirmation and restores focus',async()=>{
    const port=new BrowserDictionary();const create=vi.spyOn(port,'create').mockRejectedValueOnce(new DictionaryError('busy'));
    render(<Dictionary automaticTerms={automaticSettings()} port={port}/>);const user=userEvent.setup();const add=await screen.findByRole('button',{name:'添加词语'});await waitFor(()=>expect(add).toBeEnabled());await user.click(add);
    await user.type(screen.getByLabelText('词语'),'Figma');await user.click(screen.getByRole('button',{name:'保存'}));expect(await screen.findByRole('alert')).toHaveTextContent('正在忙');expect(screen.getByLabelText('词语')).toHaveValue('Figma');
    await user.click(screen.getByRole('button',{name:'保存'}));await waitFor(()=>expect(screen.queryByRole('dialog')).not.toBeInTheDocument());expect(create).toHaveBeenCalledTimes(2);expect(await screen.findByText('Figma')).toBeVisible();
    await user.click(screen.getByRole('button',{name:'更多操作 Figma'}));await user.click(screen.getByRole('menuitem',{name:'编辑'}));await user.type(screen.getByLabelText('词语'),'Draft');await user.click(screen.getByRole('button',{name:'取消'}));expect(await screen.findByText('Figma')).toBeVisible();expect(screen.getByRole('button',{name:'更多操作 Figma'})).toHaveFocus();
  });
  it('opens existing duplicate explicitly without overwriting',async()=>{
    const port=new BrowserDictionary();await port.create({kind:'term',original:'Figma',category:null,replacement:null});render(<Dictionary automaticTerms={automaticSettings()} port={port}/>);const user=userEvent.setup();await screen.findByText('Figma');
    await user.click(screen.getByRole('button',{name:'添加词语'}));await user.type(screen.getByLabelText('词语'),'figma');await user.click(screen.getByRole('button',{name:'保存'}));await screen.findByRole('alert');expect(screen.getByLabelText('词语')).toHaveValue('figma');
    await user.click(screen.getByRole('button',{name:'查看已有词条'}));expect(await screen.findByLabelText('词语')).toHaveValue('Figma');expect((await port.list({kind:'term',search:'',page:1})).total).toBe(1);
  });
  it('ignores stale search and unmounted requests',async()=>{
    const port=new BrowserDictionary();let oldResolve:(p:DictionaryPage)=>void=()=>{};
    const original=port.list.bind(port);vi.spyOn(port,'list').mockImplementation(q=>q.search==='old'?new Promise(resolve=>{oldResolve=resolve;}):original(q));
    const view=render(<Dictionary automaticTerms={automaticSettings()} port={port}/>);const user=userEvent.setup();await screen.findByText('还没有词条');await user.click(screen.getByRole('button',{name:'搜索词语'}));const input=screen.getByRole('textbox',{name:'搜索词条'});await user.type(input,'old');await new Promise(r=>setTimeout(r,220));await user.clear(input);await user.type(input,'new');await screen.findByText('没有匹配的词条');
    await act(async()=>oldResolve({records:[{id:'old',kind:'term',original:'旧结果',category:null,replacement:null,revision:1,createdAt:1,updatedAt:1}],page:1,total:1}));expect(screen.queryByText('旧结果')).not.toBeInTheDocument();view.unmount();
  });
  it('waits for a pending write and requires confirmation after a revision conflict',async()=>{
    const port=new BrowserDictionary();const first=await port.create({kind:'term',original:'Figma',category:null,replacement:null});
    render(<Dictionary automaticTerms={automaticSettings()} port={port}/>);const user=userEvent.setup();await screen.findByText('Figma');await user.click(screen.getByRole('button',{name:'更多操作 Figma'}));await user.click(screen.getByRole('menuitem',{name:'编辑'}));
    await port.update({kind:'term',original:'Figma Current',category:null,replacement:null},first.id,first.revision);
    await user.clear(screen.getByLabelText('词语'));await user.type(screen.getByLabelText('词语'),'My Draft');await user.click(screen.getByRole('button',{name:'保存'}));
    expect(await screen.findByRole('alert')).toHaveTextContent('记录已被修改');expect(screen.getByLabelText('词语')).toHaveValue('My Draft');
    await user.click(screen.getByRole('button',{name:'重新读取当前记录'}));await waitFor(()=>expect(screen.getByLabelText('词语')).toHaveValue('Figma Current'));
    let release:()=>void=()=>{};const update=port.update.bind(port);vi.spyOn(port,'update').mockImplementation(async(...args)=>{await new Promise<void>(resolve=>{release=resolve;});return update(...args);});
    await user.clear(screen.getByLabelText('词语'));await user.type(screen.getByLabelText('词语'),'Confirmed');await user.click(screen.getByRole('button',{name:'保存'}));
    expect(screen.getByRole('button',{name:'保存'})).toBeDisabled();await user.keyboard('{Escape}');expect(screen.getByRole('dialog')).toBeInTheDocument();expect(screen.queryByText('已保存')).not.toBeInTheDocument();
    await act(async()=>release());expect(await screen.findByText('Confirmed')).toBeVisible();
  });
});


