import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,expect,it,vi} from 'vitest';
import {Dictionary} from '../src/features/dictionary';
import {BrowserDictionary} from '../src/adapters/dictionary-browser';
import {DictionaryError,type DictionaryPage} from '../src/contracts/dictionary';
describe('manual dictionary',()=>{
  it('preserves input on failure, saves only after confirmation and restores focus',async()=>{
    const port=new BrowserDictionary();const create=vi.spyOn(port,'create').mockRejectedValueOnce(new DictionaryError('busy'));
    render(<Dictionary port={port}/>);const user=userEvent.setup();const add=await screen.findByRole('button',{name:'新增词条'});await waitFor(()=>expect(add).toBeEnabled());await user.click(add);
    await user.type(screen.getByLabelText('词条'),'Figma');await user.click(screen.getByRole('button',{name:'保存'}));expect(await screen.findByRole('alert')).toHaveTextContent('正在忙');expect(screen.getByLabelText('词条')).toHaveValue('Figma');
    await user.click(screen.getByRole('button',{name:'保存'}));await waitFor(()=>expect(screen.queryByRole('dialog')).not.toBeInTheDocument());expect(create).toHaveBeenCalledTimes(2);expect(await screen.findByRole('cell',{name:'Figma'})).toBeVisible();
    await user.click(screen.getByRole('button',{name:'编辑 Figma'}));await user.type(screen.getByLabelText('词条'),'Draft');await user.click(screen.getByRole('button',{name:'取消'}));expect(await screen.findByRole('cell',{name:'Figma'})).toBeVisible();expect(screen.getByRole('button',{name:'编辑 Figma'})).toHaveFocus();
  });
  it('opens existing duplicate explicitly without overwriting',async()=>{
    const port=new BrowserDictionary();await port.create({kind:'term',original:'Figma',category:null,replacement:null});render(<Dictionary port={port}/>);const user=userEvent.setup();await screen.findByRole('cell',{name:'Figma'});
    await user.click(screen.getByRole('button',{name:'新增词条'}));await user.type(screen.getByLabelText('词条'),'figma');await user.click(screen.getByRole('button',{name:'保存'}));await screen.findByRole('alert');expect(screen.getByLabelText('词条')).toHaveValue('figma');
    await user.click(screen.getByRole('button',{name:'查看已有词条'}));expect(await screen.findByLabelText('词条')).toHaveValue('Figma');expect((await port.list({kind:'term',search:'',page:1})).total).toBe(1);
  });
  it('ignores stale search and unmounted requests',async()=>{
    const port=new BrowserDictionary();let oldResolve:(p:DictionaryPage)=>void=()=>{};
    const original=port.list.bind(port);vi.spyOn(port,'list').mockImplementation(q=>q.search==='old'?new Promise(resolve=>{oldResolve=resolve;}):original(q));
    const view=render(<Dictionary port={port}/>);const user=userEvent.setup();await screen.findByText('还没有词条');const input=screen.getByRole('textbox',{name:'搜索词条'});await user.type(input,'old');await new Promise(r=>setTimeout(r,220));await user.clear(input);await user.type(input,'new');await screen.findByText('没有匹配的词条');
    await act(async()=>oldResolve({records:[{id:'old',kind:'term',original:'旧结果',category:null,replacement:null,revision:1,createdAt:1,updatedAt:1}],page:1,total:1}));expect(screen.queryByText('旧结果')).not.toBeInTheDocument();view.unmount();
  });
  it('waits for a pending write and requires confirmation after a revision conflict',async()=>{
    const port=new BrowserDictionary();const first=await port.create({kind:'term',original:'Figma',category:null,replacement:null});
    render(<Dictionary port={port}/>);const user=userEvent.setup();await screen.findByRole('cell',{name:'Figma'});await user.click(screen.getByRole('button',{name:'编辑 Figma'}));
    await port.update({kind:'term',original:'Figma Current',category:null,replacement:null},first.id,first.revision);
    await user.clear(screen.getByLabelText('词条'));await user.type(screen.getByLabelText('词条'),'My Draft');await user.click(screen.getByRole('button',{name:'保存'}));
    expect(await screen.findByRole('alert')).toHaveTextContent('记录已被修改');expect(screen.getByLabelText('词条')).toHaveValue('My Draft');
    await user.click(screen.getByRole('button',{name:'重新读取当前记录'}));await waitFor(()=>expect(screen.getByLabelText('词条')).toHaveValue('Figma Current'));
    let release:()=>void=()=>{};const update=port.update.bind(port);vi.spyOn(port,'update').mockImplementation(async(...args)=>{await new Promise<void>(resolve=>{release=resolve;});return update(...args);});
    await user.clear(screen.getByLabelText('词条'));await user.type(screen.getByLabelText('词条'),'Confirmed');await user.click(screen.getByRole('button',{name:'保存'}));
    expect(screen.getByRole('button',{name:'保存'})).toBeDisabled();await user.keyboard('{Escape}');expect(screen.getByRole('dialog')).toBeInTheDocument();expect(screen.queryByText('已保存')).not.toBeInTheDocument();
    await act(async()=>release());expect(await screen.findByRole('cell',{name:'Confirmed'})).toBeVisible();
  });
});


