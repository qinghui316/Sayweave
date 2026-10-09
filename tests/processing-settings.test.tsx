import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {ProcessingSettings} from '../src/features/settings';
import type {ProcessingPort,ProcessingStatus} from '../src/contracts/processing';
const initial:ProcessingStatus={settings:{enabled:true,revision:1},available:true,model:'文字服务',error:null};
describe('processing settings',()=>{
 it('saves personalization independently without changing cleanup',async()=>{
  const view={...initial,personalization:{enabled:true,revision:9}};
  const port:ProcessingPort={status:vi.fn().mockResolvedValue(view),save:vi.fn(),saveLanguage:vi.fn().mockResolvedValue({...view,personalization:{enabled:false,revision:10}})};
  const user=userEvent.setup();render(<ProcessingSettings port={port} revision={0}/>);await screen.findByRole('switch',{name:'个性化'});
  await user.click(screen.getByRole('switch',{name:'个性化'}));await user.click(screen.getByRole('button',{name:'保存整理设置'}));
  await waitFor(()=>expect(screen.queryByRole('button',{name:'保存整理设置'})).not.toBeInTheDocument());
  expect(port.save).not.toHaveBeenCalled();expect(port.saveLanguage).toHaveBeenCalledWith({enabled:false,revision:9});expect(screen.getByRole('switch',{name:'整理口述'})).toBeChecked();
 });
 it('keeps the successful first save when personalization save fails',async()=>{
  const view={...initial,personalization:{enabled:true,revision:1}},cleaned={...view,settings:{enabled:false,revision:2}};
  const port:ProcessingPort={status:vi.fn().mockResolvedValue(view),save:vi.fn().mockResolvedValue(cleaned),saveLanguage:vi.fn().mockRejectedValueOnce({code:'unavailable'}).mockResolvedValueOnce({...cleaned,personalization:{enabled:false,revision:2}})};
  const user=userEvent.setup();render(<ProcessingSettings port={port} revision={0}/>);await screen.findByRole('switch',{name:'个性化'});
  await user.click(screen.getByRole('switch',{name:'整理口述'}));await user.click(screen.getByRole('switch',{name:'个性化'}));await user.click(screen.getByRole('button',{name:'保存整理设置'}));await screen.findByText('保存失败，修改仍保留。');
  await user.click(screen.getByRole('button',{name:'保存整理设置'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'保存整理设置'})).not.toBeInTheDocument());expect(port.save).toHaveBeenCalledOnce();expect(port.saveLanguage).toHaveBeenCalledTimes(2);
 });
 it('preserves input on save failure and model metadata refresh, then protects leaving',async()=>{
  let guard:((next:()=>void)=>void)|null=null;const bind=(value:typeof guard)=>{guard=value;};
  const port:ProcessingPort={status:vi.fn().mockResolvedValue(initial),save:vi.fn().mockRejectedValue({code:'unavailable'})};const user=userEvent.setup();
  const view=render(<ProcessingSettings port={port} revision={0} onGuardChange={bind}/>);await screen.findByText('文字模型：文字服务');
  await user.click(screen.getByRole('switch',{name:'整理口述'}));await user.click(screen.getByRole('button',{name:'保存整理设置'}));await screen.findByText('保存失败，修改仍保留。');expect(screen.getByRole('switch')).not.toBeChecked();
  vi.mocked(port.status).mockResolvedValue({...initial,model:'新文字服务'});view.rerender(<ProcessingSettings port={port} revision={1} onGuardChange={bind}/>);await screen.findByText('文字模型：新文字服务');expect(screen.getByRole('switch')).not.toBeChecked();
  const leave=vi.fn();await waitFor(()=>expect(guard).not.toBeNull());act(()=>guard!(leave));await screen.findByRole('alertdialog');await user.click(screen.getByRole('button',{name:'继续填写'}));expect(leave).not.toHaveBeenCalled();
  act(()=>guard!(leave));await user.click(screen.getByRole('button',{name:'放弃修改'}));expect(leave).toHaveBeenCalledOnce();
 });
 it('saves only the explicit preference using its revision',async()=>{
  const port:ProcessingPort={status:vi.fn().mockResolvedValue(initial),save:vi.fn().mockResolvedValue({...initial,settings:{enabled:false,revision:2}})};const user=userEvent.setup();
  render(<ProcessingSettings port={port} revision={0}/>);await screen.findByText('文字模型：文字服务');await user.click(screen.getByRole('switch'));await user.click(screen.getByRole('button',{name:'保存整理设置'}));
  await waitFor(()=>expect(screen.queryByRole('button',{name:'保存整理设置'})).not.toBeInTheDocument());expect(port.save).toHaveBeenCalledWith({enabled:false,revision:1});expect(screen.getByRole('switch')).not.toBeChecked();
 });
 it('rejects status reads started before and during a successful save',async()=>{
  const deferred=()=>{let resolve!:(value:ProcessingStatus)=>void;return {promise:new Promise<ProcessingStatus>(r=>{resolve=r;}),resolve:(value:ProcessingStatus)=>resolve(value)};};
  const old=deferred(),during=deferred(),saved=deferred();
  const port:ProcessingPort={status:vi.fn().mockResolvedValueOnce(initial).mockReturnValueOnce(old.promise).mockReturnValueOnce(during.promise),save:vi.fn().mockReturnValueOnce(saved.promise).mockResolvedValueOnce({...initial,settings:{enabled:true,revision:3}})};
  const user=userEvent.setup(),view=render(<ProcessingSettings port={port} revision={0}/>);await screen.findByText('文字模型：文字服务');
  view.rerender(<ProcessingSettings port={port} revision={1}/>);await waitFor(()=>expect(port.status).toHaveBeenCalledTimes(2));
  await user.click(screen.getByRole('switch'));await user.click(screen.getByRole('button',{name:'保存整理设置'}));
  view.rerender(<ProcessingSettings port={port} revision={2}/>);await waitFor(()=>expect(port.status).toHaveBeenCalledTimes(3));
  await act(async()=>{saved.resolve({...initial,settings:{enabled:false,revision:2}});});
  await act(async()=>{during.resolve(initial);old.resolve(initial);});
  expect(screen.getByRole('switch')).not.toBeChecked();expect(screen.queryByRole('button',{name:'保存整理设置'})).not.toBeInTheDocument();
  await user.click(screen.getByRole('switch'));await user.click(screen.getByRole('button',{name:'保存整理设置'}));
  await waitFor(()=>expect(port.save).toHaveBeenLastCalledWith({enabled:true,revision:2}));
 });
});
