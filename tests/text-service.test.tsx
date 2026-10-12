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
  await waitFor(()=>expect(screen.getByRole('switch',{name:'启用原位输入'})).toBeEnabled());await user.click(screen.getByRole('switch',{name:'启用原位输入'}));
  await waitFor(()=>expect(screen.getByRole('switch')).toBeChecked());
  await user.click(screen.getByRole('button',{name:'组件管理'}));await user.click(screen.getByRole('menuitem',{name:'检查状态'}));expect(action).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole('switch'));
  await waitFor(()=>expect(screen.getByRole('switch')).toBeEnabled());await user.click(screen.getByRole('switch'));
  await user.click(screen.getByRole('button',{name:'组件管理'}));
  await user.click(screen.getByRole('menuitem',{name:'移除组件'}));
  expect(action).not.toHaveBeenCalledWith('remove');
  await user.click(screen.getByRole('button',{name:'移除'}));
  await waitFor(()=>expect(screen.getByRole('switch')).not.toBeChecked());
  expect(action.mock.calls.map(c=>c[0])).toEqual(['install','disable','enable','remove']);
 });
 it('retains the saved status after a failed action and ignores late replies after unmount',async()=>{
  const value:TextServiceStatus={installed:true,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:false,deploymentState:'ready',error:null};
  let resolve!:(v:TextServiceStatus)=>void;
  const port:TextServicePort={mode:'native',status:async()=>value,act:vi.fn().mockRejectedValueOnce({code:'componentEnableFailed'}).mockImplementationOnce(()=>new Promise<TextServiceStatus>(r=>{resolve=r;}))};
  const view=render(<TextServiceSettings port={port}/>),user=userEvent.setup();
  await waitFor(()=>expect(screen.getByRole('switch')).toBeEnabled());await user.click(screen.getByRole('switch'));
  await screen.findByText('未能启用输入组件，请重试。');
  expect(screen.getByRole('switch')).toBeEnabled();
  await user.click(screen.getByRole('switch'));
  await waitFor(()=>expect(screen.getByRole('switch')).toBeDisabled());
  view.unmount();await act(async()=>{resolve({...value,enabled:true});});
 });
 it('rejects impossible native state combinations',()=>{
  expect(()=>parseTextService({installed:false,enabled:true,updateAvailable:false,payloadAvailable:true,cleanupPending:false,deploymentState:'ready',error:null})).toThrow();
  expect(()=>parseTextService({installed:true,enabled:true,updateAvailable:true,payloadAvailable:false,cleanupPending:false,deploymentState:'ready',error:null})).toThrow();
 });
 it('keeps repair available when an owned registration has lost its module',async()=>{
  const damaged:TextServiceStatus={installed:true,enabled:false,updateAvailable:true,payloadAvailable:true,cleanupPending:false,deploymentState:'required',error:'componentRegistration'};
  const restored:TextServiceStatus={...damaged,enabled:true,updateAvailable:false,error:null};
  const port:TextServicePort={mode:'native',status:async()=>damaged,act:vi.fn().mockResolvedValue(restored)};
  render(<TextServiceSettings port={port}/>);const user=userEvent.setup();
  await user.click(screen.getByRole('button',{name:'组件管理'}));await user.click(await screen.findByRole('menuitem',{name:'修复组件'}));
  expect(port.act).toHaveBeenCalledWith('install');
  await waitFor(()=>expect(screen.getByRole('switch')).toBeChecked());
  expect(screen.queryByRole('menuitem',{name:'修复组件'})).not.toBeInTheDocument();
 });
 it('offers exact cleanup again after registration removal leaves a mapped file',async()=>{
  const value:TextServiceStatus={installed:false,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:true,deploymentState:'required',error:null};
  const port:TextServicePort={mode:'native',status:async()=>value,act:vi.fn().mockResolvedValue({...value,cleanupPending:false})};
  render(<TextServiceSettings port={port}/>);const user=userEvent.setup();
  await user.click(screen.getByRole('button',{name:'组件管理'}));
  await user.click(screen.getByRole('menuitem',{name:'重试清理'}));
  expect(port.act).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button',{name:'移除'}));
  await waitFor(()=>expect(screen.getByRole('switch')).toBeEnabled());
  expect(port.act).toHaveBeenCalledWith('remove');
 });
 it.each(['pending','failed'] as const)('keeps an unfinished %s deployment under installer recovery ownership',async(deploymentState)=>{
  const value:TextServiceStatus={installed:true,enabled:true,updateAvailable:true,payloadAvailable:true,cleanupPending:false,deploymentState,error:null};
  const port:TextServicePort={mode:'native',status:async()=>value,act:vi.fn()};
  render(<TextServiceSettings port={port}/>);const user=userEvent.setup();
  await waitFor(()=>expect(screen.getByRole('switch')).toBeChecked());
  expect(screen.getByRole('switch')).toBeDisabled();
  await user.click(screen.getByRole('button',{name:'组件管理'}));
  expect(screen.queryByRole('menuitem',{name:'修复组件'})).not.toBeInTheDocument();
  expect(screen.getByRole('menuitem',{name:'移除组件'})).toHaveAttribute('aria-disabled','true');
  expect(port.act).not.toHaveBeenCalled();
 });
});
