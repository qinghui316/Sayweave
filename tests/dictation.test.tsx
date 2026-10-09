import {shortcutSettings} from './settings-fixtures';
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
 it('keeps submission and readback separate and rejects unknown outcomes',()=>{
  const delivery={state:'unconfirmed',submission:'submitted',verification:'unavailable',stage:'verify',submittedEvents:4,error:'inputUnconfirmed',restoreError:false,elapsedMs:500};
  expect(parseDictation({...initialDictation(),delivery}).delivery).toEqual(delivery);
  expect(()=>parseDictation({...initialDictation(),delivery:{...delivery,submission:'confirmed'}})).toThrow();
  expect(()=>parseDictation({...initialDictation(),delivery:{...delivery,verification:'assumed'}})).toThrow();
  expect(()=>parseDictation({...initialDictation(),delivery:{...delivery,submittedEvents:-1}})).toThrow();
 });
});
describe('shortcut settings',()=>{
 it('saves a completed capture RPC even when the subscription has not attached',async()=>{
  const base=new BrowserShortcuts(),save=vi.spyOn(base,'save');let attach!:()=>void;
  const port:ShortcutConfigPort={mode:'simulation',status:()=>base.status(),save:value=>base.save(value),subscribe:()=>new Promise(resolve=>{attach=()=>resolve(()=>{});}),capture:async(begin,token)=>{const value=await base.capture(begin,token);return begin?{...value,capturing:false,capturedBinding:'Control+KeyD',sequence:value.sequence+1}:value;}};
  const user=userEvent.setup(),view=render(<ShortcutSettings port={port} settings={shortcutSettings(port)}/>);await screen.findByText('已启用：右 Alt');await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  await waitFor(()=>expect(save).toHaveBeenCalledWith({binding:'Control+KeyD',mode:'toggle',revision:1}));view.unmount();await act(async()=>attach());
 });
 it('keeps the current capture owned when an unrelated initial read arrives before capture starts',async()=>{
  const base=new BrowserShortcuts();let resolveRead!:(value:ShortcutStatus)=>void,startCapture!:(()=>Promise<void>);
  const port:ShortcutConfigPort={mode:'simulation',subscribe:cb=>base.subscribe(cb),save:v=>base.save(v),status:()=>new Promise(resolve=>{resolveRead=resolve;}),capture:(begin,token)=>begin?new Promise(resolve=>{startCapture=async()=>{resolve(await base.capture(true,token));};}):base.capture(false)};
  const user=userEvent.setup();render(<ShortcutSettings settings={shortcutSettings(port)} port={port}/>);
  await act(async()=>{await base.capture(false);});await screen.findByText('已启用：右 Alt');
  // A newer initial read is sampled before this editor's capture command reaches the backend.
  const unrelated={...await base.status(),captureToken:crypto.randomUUID(),capturedBinding:'Control+KeyX'};
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  await act(async()=>{resolveRead(unrelated);});
  expect(screen.getByRole('button',{name:'更改快捷键'})).not.toHaveTextContent('Ctrl + X');expect(screen.queryByRole('button',{name:'保存快捷键'})).not.toBeInTheDocument();
  await act(async()=>{await startCapture();});fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});
  await waitFor(()=>expect(screen.getByRole('button',{name:'更改快捷键'})).toHaveTextContent('Ctrl + D'));expect(screen.queryByRole('button',{name:'保存快捷键'})).not.toBeInTheDocument();
 });
 it('keeps an automatically saved captured key after reopening',async()=>{
  const port=new BrowserShortcuts(),settings=shortcutSettings(port),user=userEvent.setup();const view=render(<ShortcutSettings settings={settings} port={port}/>);
  await screen.findByText('已启用：右 Alt');await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});await screen.findByText('已启用：Ctrl + D');view.unmount();
  render(<ShortcutSettings settings={settings} port={port}/>);await screen.findByText('已启用：Ctrl + D');expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
 });
 it('ignores a capture response and initial read that arrive after newer completion events',async()=>{
  const base=new BrowserShortcuts();let resolveRead!:(value:ShortcutStatus)=>void,resolveCapture!:(value:ShortcutStatus)=>void;
  const initial=await base.status();let captured:ShortcutStatus|undefined;
  const port:ShortcutConfigPort={mode:'simulation',subscribe:cb=>base.subscribe(cb),save:v=>base.save(v),status:()=>new Promise(resolve=>{resolveRead=resolve;}),capture:async (begin,token)=>{const v=await base.capture(begin,token);if(!begin)return v;captured=v;return new Promise(resolve=>{resolveCapture=resolve;});}};
  const user=userEvent.setup();render(<ShortcutSettings settings={shortcutSettings(port)} port={port}/>);
  act(()=>{void base.capture(false);});await screen.findByText('已启用：右 Alt');
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));await waitFor(()=>expect(captured).toBeDefined());
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});await waitFor(()=>expect(screen.getByRole('button',{name:'更改快捷键'})).toHaveTextContent('Ctrl + D'));
  await act(async()=>{resolveCapture(captured!);resolveRead(initial);});
  expect(screen.getByRole('button',{name:'更改快捷键'})).toBeVisible();expect(screen.queryByRole('button',{name:'保存快捷键'})).not.toBeInTheDocument();
  expect(screen.getByRole('button',{name:'更改快捷键'})).toHaveTextContent('Ctrl + D');
 });
 it('records a key, saves its mode, and releases capture on unmount',async()=>{
  const port=new BrowserShortcuts(),capture=vi.spyOn(port,'capture');
  const user=userEvent.setup();const view=render(<ShortcutSettings settings={shortcutSettings(port)} port={port}/>);
  await screen.findByText('已启用：右 Alt');
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));
  fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});
  await waitFor(()=>expect(screen.getByRole('button',{name:'更改快捷键'})).toHaveTextContent('Ctrl + D'));
  await user.click(screen.getByRole('radio',{name:'按住'}));
  await screen.findByText('已启用：Ctrl + D');
  await waitFor(async()=>expect((await port.status()).saved).toMatchObject({binding:'Control+KeyD',mode:'hold'}));
  await user.click(screen.getByRole('button',{name:'更改快捷键'}));view.unmount();expect(capture).toHaveBeenLastCalledWith(false,expect.any(String));
 });
 it('shows save failure without a leave prompt or an uncommitted mode',async()=>{
  const base=new BrowserShortcuts();const port:ShortcutConfigPort={mode:'simulation',status:()=>base.status(),subscribe:cb=>base.subscribe(cb),capture:(b,t)=>base.capture(b,t),save:vi.fn().mockRejectedValue({code:'shortcutConflict'})};
  const user=userEvent.setup();render(<ShortcutSettings settings={shortcutSettings(port)} port={port}/>);await screen.findByText('已启用：右 Alt');await user.click(screen.getByRole('radio',{name:'按住'}));
  await screen.findByText('此快捷键已被占用，请换一个。');expect(screen.getByRole('radio',{name:'点按'})).toBeChecked();expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
 });
});
it('keeps external completion pending only for the matching nonempty successful task',()=>{
 const result={...initialDictation(),taskId:'A',source:'external' as const,stage:'result' as const,text:'你好',presentationId:'view-A'};
 const speech={taskId:'A',text:'你好'};
 expect(dictationPhase(result,'completed',speech)).toBe('processing');
 expect(dictationPhase({...result,delivery:{state:'confirmed',submission:'submitted',verification:'confirmed',stage:'verify',submittedEvents:4,error:null,restoreError:false,elapsedMs:10}},'completed',speech)).toBe('completed');
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

it('shows only recognition feedback while final text is pending and clears completed captions',()=>{
 const view=render(<VoiceSurface presentation="recognizing" phase="processing" text="尾字尚在返回" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.getByRole('group',{name:'识别中'})).toBeVisible();expect(screen.queryByText('识别中')).not.toBeInTheDocument();expect(screen.queryByLabelText('实时字幕')).not.toBeInTheDocument();
 view.rerender(<VoiceSurface presentation="completed" phase="completed" text="完整结果" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.getByRole('group',{name:'口述完成'})).toBeVisible();expect(screen.queryByText('口述完成')).not.toBeInTheDocument();expect(screen.queryByText('完整结果')).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'复制'})).not.toBeInTheDocument();
});
it('terminal input notices render no explanatory banner or actions',()=>{
 render(<VoiceSurface presentation="notice" phase="completed" text="已填入文字" delivery={{state:'unconfirmed',submission:'submitted',verification:'timeout',stage:'verify',submittedEvents:4,error:'compositionRejected',restoreError:false,elapsedMs:1000}} start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={()=>{}} copy={async()=>{}}/>);
 expect(screen.queryByRole('region',{name:'口述提示'})).not.toBeInTheDocument();
 expect(screen.queryByText('草稿更新已停止，请检查输入框。')).not.toBeInTheDocument();
 expect(screen.queryByRole('button')).not.toBeInTheDocument();
});
it('no-target result retains its content and close action after copy failure',async()=>{
 const dismiss=vi.fn(),copy=vi.fn().mockRejectedValue(Error('clipboard busy')),user=userEvent.setup();
 render(<VoiceSurface presentation="result" phase="completed" text="完整结果" start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={dismiss} copy={copy}/>);
 await user.click(screen.getByRole('button',{name:'复制'}));await screen.findByRole('button',{name:'复制失败，重试'});
 expect(screen.getByText('完整结果')).toBeVisible();await user.click(screen.getByRole('button',{name:'关闭'}));expect(dismiss).toHaveBeenCalledOnce();
});
it('dismissed external feedback does not replay its completed state when shown later',()=>{
 expect(dictationPhase({...initialDictation(),source:'external',taskId:'old',stage:'result',text:'old'},'completed',{taskId:'old',text:'old'})).toBe('idle');
 expect(()=>parseDictation({...initialDictation(),presentation:'recording'})).toThrow();
});

it.each(['editing','resuming'] as const)('keeps finish available while the continuous session is %s without a result card',async presentation=>{
 const finish=vi.fn(),user=userEvent.setup();
 const state=parseDictation({...initialDictation(),source:'external',taskId:'live',presentationId:'view',stage:'recognizing',presentation});
 const phase=dictationPhase(state,'recording',{taskId:'live',text:'原始识别'});
 render(<VoiceSurface presentation={presentation} phase={phase} text="原始识别" start={()=>{}} finish={finish} cancel={()=>{}}/>);
 expect(screen.getByRole('group',{name:presentation==='editing'?'编辑中，口述已暂停':'准备继续口述'})).toBeVisible();expect(screen.queryByText(presentation==='editing'?'编辑中，口述已暂停':'准备继续口述')).not.toBeInTheDocument();
 expect(screen.queryByRole('region',{name:'口述结果'})).not.toBeInTheDocument();
 await user.click(screen.getByRole('button',{name:'结束录音'}));expect(finish).toHaveBeenCalledOnce();
});

it('dismissed preparation failures and clipboard notices remain closed when the overlay reopens',()=>{
 const dismissed={...initialDictation(),source:'external' as const,taskId:'failed',error:'network'};
 expect(dictationPhase(dismissed,'failed',{taskId:'failed',text:''})).toBe('idle');
 const view=render(<VoiceSurface presentation="idle" phase="idle" error="连接失败" delivery={{state:'confirmed',submission:'submitted',verification:'confirmed',stage:'verify',submittedEvents:4,error:null,restoreError:true,elapsedMs:10}} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('region',{name:'口述提示'})).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'重试恢复剪贴板'})).not.toBeInTheDocument();
 view.rerender(<VoiceSurface presentation="idle" phase={dictationPhase(dismissed,'failed',{taskId:'failed',text:''})} error="连接失败" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByText('连接失败')).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'重新开始录音'})).not.toBeInTheDocument();
});

it('records from the key field and cancels capture on outside interaction or window blur',async()=>{
 const port=new BrowserShortcuts(),user=userEvent.setup();render(<><ShortcutSettings settings={shortcutSettings(port)} port={port}/><button>其他设置</button></>);
 await screen.findByText('已启用：右 Alt');const field=screen.getByRole('button',{name:'更改快捷键'});
 field.focus();await user.keyboard('{Enter}');await screen.findByText('按下快捷键…');
 await user.click(screen.getByRole('button',{name:'其他设置'}));await waitFor(()=>expect(field).toHaveAttribute('aria-pressed','false'));
 expect(field).toHaveTextContent('右 Alt');expect(screen.queryByRole('button',{name:'保存快捷键'})).not.toBeInTheDocument();
 await user.click(field);await screen.findByText('按下快捷键…');fireEvent(window,new Event('blur'));
 await waitFor(()=>expect(field).toHaveAttribute('aria-pressed','false'));
 expect(field).toHaveTextContent('右 Alt');
});

it('cancels a delayed capture start when the settings window loses focus',async()=>{
 const base=new BrowserShortcuts();let start!:()=>Promise<void>;
 const port:ShortcutConfigPort={mode:'simulation',status:()=>base.status(),subscribe:cb=>base.subscribe(cb),save:v=>base.save(v),capture:(begin,token)=>begin?new Promise(resolve=>{start=async()=>resolve(await base.capture(true,token));}):base.capture(false)};
 const user=userEvent.setup();render(<ShortcutSettings settings={shortcutSettings(port)} port={port}/>);await screen.findByText('已启用：右 Alt');
 await user.click(screen.getByRole('button',{name:'更改快捷键'}));fireEvent(window,new Event('blur'));
 await act(start);await waitFor(async()=>expect((await base.status()).capturing).toBe(false));
 fireEvent.keyDown(window,{key:'d',code:'KeyD',ctrlKey:true});expect(screen.getByRole('button',{name:'更改快捷键'})).toHaveTextContent('右 Alt');
 expect(screen.queryByRole('button',{name:'保存快捷键'})).not.toBeInTheDocument();
});

it('reports rejected live admission without a copy card and exposes settings',async()=>{
 const settings=vi.fn(),user=userEvent.setup();
 const state=parseDictation({...initialDictation(),source:'external',inputRoute:'live',admission:'rejected',taskId:'task',presentationId:'view',presentation:'error',error:'componentNotEnabled'});
 render(<VoiceSurface presentation={state.presentation} phase="failed" error="请在设置中启用实时输入组件。" configure={settings} dismiss={()=>{}} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('region',{name:'口述结果'})).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'复制'})).not.toBeInTheDocument();
 await user.click(screen.getByRole('button',{name:'设置'}));expect(settings).toHaveBeenCalledOnce();
 expect(()=>parseDictation({...state,inputRoute:'paste'})).toThrow();
 expect(()=>parseDictation({...state,admission:'unknown'})).toThrow();
});
it('does not display an archive explanation over the completed target',()=>{
 render(<VoiceSurface presentation="notice" phase="completed" start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={()=>{}}/>);
 expect(screen.queryByRole('status')).not.toBeInTheDocument();
 expect(screen.queryByRole('region',{name:'口述结果'})).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'关闭'})).not.toBeInTheDocument();
});
