import {act,render,screen,waitFor,fireEvent} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {BrowserShortcuts} from '../src/adapters/dictation-tauri';
import {dictationPhase,parseDictation,parseShortcut,initialDictation,shortcutLabel,type ShortcutConfigPort,type ShortcutStatus} from '../src/contracts/dictation';
import {parseDeliveryReports} from '../src/contracts/history';
import {ShortcutSettings} from '../src/features/settings/ShortcutSettings';
import {VoiceSurface} from '../src/features/voice';
describe('dictation contracts',()=>{
 it('rejects malformed state and exposes no native target identity',()=>{
  expect(parseDictation(initialDictation()).source).toBe('trial');
  expect(()=>parseDictation({...initialDictation(),delivery:{state:'submitted',error:null,restoreError:false,elapsedMs:1}})).toThrow();
  expect(()=>parseShortcut({saved:{binding:'AltRight',mode:'toggle',revision:0}})).toThrow();
  expect(()=>parseDeliveryReports([{attemptId:'a',state:'replay',error:null,updatedAt:1}])).toThrow();
  expect(shortcutLabel('Control+Alt+KeyD')).toBe('Ctrl + Alt + D');
 });
});
describe('shortcut settings',()=>{
 it('keeps the current capture owned when an unrelated initial read arrives before capture starts',async()=>{
  const base=new BrowserShortcuts();let resolveRead!:(value:ShortcutStatus)=>void,startCapture!:(()=>Promise<void>);
  const port:ShortcutConfigPort={mode:'simulation',subscribe:cb=>base.subscribe(cb),save:v=>base.save(v),status:()=>new Promise(resolve=>{resolveRead=resolve;}),capture:(begin,token)=>begin?new Promise(resolve=>{startCapture=async()=>{resolve(await base.capture(true,token));};}):base.capture(false)};
  const user=userEvent.setup();render(<ShortcutSettings port={port}/>);
  await act(async()=>{await base.capture(false);});await screen.findByText('已启用：右 Alt');
  // A newer initial read is sampled before this editor's capture command reaches the backend.
  const unrelated={...await base.status(),captureToken:crypto.randomUUID(),capturedBinding:'Control+KeyX'};
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  await act(async()=>{resolveRead(unrelated);});
  expect(screen.queryByText('Ctrl + X')).not.toBeInTheDocument();expect(screen.getByRole('button',{name:'保存快捷键'})).toBeDisabled();
  await act(async()=>{await startCapture();});fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});
  await screen.findByText('Ctrl + D');expect(screen.getByRole('button',{name:'保存快捷键'})).toBeEnabled();
 });
 it('keeps a discarded captured key out of a reopened editor and can record the same key again',async()=>{
  const port=new BrowserShortcuts(),user=userEvent.setup();let guard:((action:()=>void)=>void)|null=null;const closed=vi.fn();
  const view=render(<ShortcutSettings port={port} onGuardChange={g=>{guard=g;}}/>);
  await screen.findByText('已启用：右 Alt');await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});await screen.findByText('Ctrl + D');
  act(()=>guard!(closed));await screen.findByRole('alertdialog',{name:'快捷键尚未保存'});
  await user.click(screen.getByRole('button',{name:'放弃修改'}));expect(closed).toHaveBeenCalledOnce();view.unmount();
  render(<ShortcutSettings port={port}/>);await screen.findByText('已启用：右 Alt');
  expect(screen.queryByText('Ctrl + D')).not.toBeInTheDocument();expect(screen.getByRole('button',{name:'保存快捷键'})).toBeDisabled();
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});
  await screen.findByText('Ctrl + D');expect(screen.getByRole('button',{name:'保存快捷键'})).toBeEnabled();
 });
 it('ignores a capture response and initial read that arrive after newer completion events',async()=>{
  const base=new BrowserShortcuts();let resolveRead!:(value:ShortcutStatus)=>void,resolveCapture!:(value:ShortcutStatus)=>void;
  const initial=await base.status();let captured:ShortcutStatus|undefined;
  const port:ShortcutConfigPort={mode:'simulation',subscribe:cb=>base.subscribe(cb),save:v=>base.save(v),status:()=>new Promise(resolve=>{resolveRead=resolve;}),capture:async (begin,token)=>{const v=await base.capture(begin,token);if(!begin)return v;captured=v;return new Promise(resolve=>{resolveCapture=resolve;});}};
  const user=userEvent.setup();render(<ShortcutSettings port={port}/>);
  act(()=>{void base.capture(false);});await screen.findByText('已启用：右 Alt');
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));await waitFor(()=>expect(captured).toBeDefined());
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});await screen.findByText('Ctrl + D');
  await act(async()=>{resolveCapture(captured!);resolveRead(initial);});
  expect(screen.getByRole('button',{name:'更改快捷键'})).toBeVisible();expect(screen.getByRole('button',{name:'保存快捷键'})).toBeEnabled();
  expect(screen.getByText('Ctrl + D')).toBeVisible();
 });
 it('records a key, saves its mode, and releases capture on unmount',async()=>{
  const port=new BrowserShortcuts(),capture=vi.spyOn(port,'capture');
  const user=userEvent.setup();const view=render(<ShortcutSettings port={port}/>);
  await screen.findByText('已启用：右 Alt');
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});
  await screen.findByText('Ctrl + D');
  await user.selectOptions(screen.getByRole('combobox',{name:'录音方式'}),'hold');
  await user.click(screen.getByRole('button',{name:'保存快捷键'}));
  await screen.findByText('已启用：Ctrl + D');
  expect((await port.status()).saved).toEqual({binding:'Control+KeyD',mode:'hold',revision:2});
  view.unmount();expect(capture).toHaveBeenLastCalledWith(false);
 });
 it('keeps a failed draft and protects closing until discard',async()=>{
  const base=new BrowserShortcuts();const port:ShortcutConfigPort={mode:'simulation',status:()=>base.status(),subscribe:cb=>base.subscribe(cb),capture:(b,t)=>base.capture(b,t),save:vi.fn().mockRejectedValue({code:'shortcutConflict'})};
  let guard:((action:()=>void)=>void)|null=null;const closed=vi.fn(),user=userEvent.setup();
  render(<ShortcutSettings port={port} onGuardChange={g=>{guard=g;}}/>);
  await screen.findByText('已启用：右 Alt');
  await user.selectOptions(screen.getByRole('combobox',{name:'录音方式'}),'hold');
  await user.click(screen.getByRole('button',{name:'保存快捷键'}));
  await screen.findByText('此快捷键已被占用，请换一个。');
  expect(screen.getByRole('combobox')).toHaveValue('hold');
  fireEvent.click(screen.getByRole('button',{name:'更改快捷键'}));
  await waitFor(()=>expect(guard).not.toBeNull());
  guard!(closed);await screen.findByRole('alertdialog',{name:'快捷键尚未保存'});
  expect(closed).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button',{name:'放弃修改'}));expect(closed).toHaveBeenCalledOnce();
 });
});
it('separates confirmed input from a failed clipboard restore',async()=>{
 const restore=vi.fn().mockResolvedValue(undefined),user=userEvent.setup();
 render(<VoiceSurface phase="idle" text="你好" delivery={{state:'confirmed',error:null,restoreError:true,elapsedMs:10}} start={()=>{}} finish={()=>{}} cancel={()=>{}} restore={restore}/>);
 expect(screen.getByText('已填入')).toBeVisible();
 await user.click(screen.getByRole('button',{name:'重试恢复剪贴板'}));
 await waitFor(()=>expect(screen.queryByText('剪贴板恢复未完成。')).not.toBeInTheDocument());
 expect(restore).toHaveBeenCalledOnce();
});

it('keeps external completion pending only for the matching nonempty successful task',()=>{
 const result={...initialDictation(),taskId:'A',source:'external' as const,stage:'result' as const,text:'你好'};
 const speech={taskId:'A',text:'你好'};
 expect(dictationPhase(result,'completed',speech)).toBe('processing');
 expect(dictationPhase({...result,delivery:{state:'confirmed',error:null,restoreError:false,elapsedMs:10}},'completed',speech)).toBe('completed');
 expect(dictationPhase({...result,stage:'delivering'},'completed',speech)).toBe('processing');
 expect(dictationPhase(result,'failed',speech)).toBe('failed');
 expect(dictationPhase(result,'completed',{...speech,taskId:'old'})).toBe('completed');
 expect(dictationPhase(result,'completed',{...speech,text:''})).toBe('completed');
 expect(dictationPhase({...result,error:'interrupted'},'processing',speech)).toBe('failed');
 expect(dictationPhase(initialDictation(),'completed',speech)).toBe('completed');
});

it('offers stopping delivery after recognition without a discard label',async()=>{
 const cancel=vi.fn(),user=userEvent.setup();
 render(<VoiceSurface phase="processing" text="已识别文字" cancelLabel="停止填入" start={()=>{}} finish={()=>{}} cancel={cancel}/>);
 await user.click(screen.getByRole('button',{name:'停止填入'}));
 expect(cancel).toHaveBeenCalledOnce();
 expect(screen.queryByRole('button',{name:'取消并丢弃'})).not.toBeInTheDocument();
});
