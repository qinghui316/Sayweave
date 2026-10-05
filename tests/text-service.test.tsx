import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,expect,it,vi} from 'vitest';
import {BrowserTextService} from '../src/adapters/dictation-tauri';
import {TextServiceSettings} from '../src/features/settings/TextServiceSettings';
import {parseTextService,type TextServicePort,type TextServiceStatus} from '../src/contracts/text-service';
describe('component lifecycle',()=>{
 it('installs, disables, reenables and confirms removal with fixed operations',async()=>{
  const port=new BrowserTextService(),action=vi.spyOn(port,'act'),user=userEvent.setup();
  render(<TextServiceSettings port={port}/>);
  await user.click(await screen.findByRole('button',{name:'安装并启用'}));
  await screen.findByRole('button',{name:'检查状态'});
  await user.click(screen.getByRole('button',{name:'检查状态'}));expect(action).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole('button',{name:'停用'}));
  await user.click(await screen.findByRole('button',{name:'启用'}));
  await user.click(await screen.findByText('组件管理'));
  await user.click(screen.getByRole('button',{name:'移除组件'}));
  expect(action).not.toHaveBeenCalledWith('remove');
  await user.click(screen.getByRole('button',{name:'移除'}));
  await screen.findByRole('button',{name:'安装并启用'});
  expect(action.mock.calls.map(c=>c[0])).toEqual(['install','disable','enable','remove']);
 });
 it('retains the saved status after a failed action and ignores late replies after unmount',async()=>{
  const value:TextServiceStatus={installed:true,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:false,error:null};
  let resolve!:(v:TextServiceStatus)=>void;
  const port:TextServicePort={mode:'native',status:async()=>value,act:vi.fn().mockRejectedValueOnce({code:'componentEnableFailed'}).mockImplementationOnce(()=>new Promise<TextServiceStatus>(r=>{resolve=r;}))};
  const view=render(<TextServiceSettings port={port}/>),user=userEvent.setup();
  await user.click(await screen.findByRole('button',{name:'启用'}));
  await screen.findByText('未能启用输入组件，请重试。');
  expect(screen.getByRole('button',{name:'启用'})).toBeEnabled();
  await user.click(screen.getByRole('button',{name:'启用'}));
  await waitFor(()=>expect(screen.getByRole('button',{name:'处理中…'})).toBeDisabled());
  view.unmount();await act(async()=>{resolve({...value,enabled:true});});
 });
 it('rejects impossible native state combinations',()=>{
  expect(()=>parseTextService({installed:false,enabled:true,updateAvailable:false,payloadAvailable:true,cleanupPending:false,error:null})).toThrow();
  expect(()=>parseTextService({installed:true,enabled:true,updateAvailable:true,payloadAvailable:false,cleanupPending:false,error:null})).toThrow();
 });
 it('keeps repair available when an owned registration has lost its module',async()=>{
  const damaged:TextServiceStatus={installed:true,enabled:false,updateAvailable:true,payloadAvailable:true,cleanupPending:false,error:'componentRegistration'};
  const restored:TextServiceStatus={...damaged,enabled:true,updateAvailable:false,error:null};
  const port:TextServicePort={mode:'native',status:async()=>damaged,act:vi.fn().mockResolvedValue(restored)};
  render(<TextServiceSettings port={port}/>);const user=userEvent.setup();
  await user.click(await screen.findByRole('button',{name:'更新组件'}));
  expect(port.act).toHaveBeenCalledWith('install');
  await screen.findByRole('button',{name:'检查状态'});
  expect(screen.queryByRole('button',{name:'更新组件'})).not.toBeInTheDocument();
 });
 it('offers exact cleanup again after registration removal leaves a mapped file',async()=>{
  const value:TextServiceStatus={installed:false,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:true,error:null};
  const port:TextServicePort={mode:'native',status:async()=>value,act:vi.fn().mockResolvedValue({...value,cleanupPending:false})};
  render(<TextServiceSettings port={port}/>);const user=userEvent.setup();
  await user.click(await screen.findByText('组件管理'));
  await user.click(screen.getByRole('button',{name:'重试清理'}));
  expect(port.act).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button',{name:'移除'}));
  await waitFor(()=>expect(screen.queryByText('组件管理')).not.toBeInTheDocument());
  expect(port.act).toHaveBeenCalledWith('remove');
 });
});
