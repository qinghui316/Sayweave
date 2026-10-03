import {act,fireEvent,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {ModelPanel} from '../src/features/models';
import {BrowserModels} from '../src/adapters/models-browser';
import {ModelError,type ModelConfigPort,type ModelTestResult} from '../src/contracts/models';
async function panel(port:ModelConfigPort=new BrowserModels(),onGuardChange?:((guard:((action:()=>void)=>void)|null)=>void)){
 render(<ModelPanel port={port} onGuardChange={onGuardChange}/>);
 await screen.findByRole('heading',{name:'添加 API 模型'});
 return port;
}
async function fill(){
 fireEvent.change(screen.getByLabelText('配置名称'),{target:{value:'个人配置'}});
 fireEvent.change(screen.getByLabelText('服务基址'),{target:{value:'https://example.invalid/v1'}});
 fireEvent.change(screen.getByLabelText('API Key'),{target:{value:'dummy-personal-key'}});
 fireEvent.change(screen.getByLabelText('模型 ID'),{target:{value:'test-model'}});
}
describe('inline model panel',()=>{
 it('opens form, saves without network calls, edits and confirms deletion',async()=>{
  const p=new BrowserModels();const fetch=vi.spyOn(p,'fetchModels'),test=vi.spyOn(p,'testTextModel');
  await panel(p);await fill();fireEvent.click(screen.getByRole('button',{name:'保存配置'}));
  await screen.findByText('配置已保存。');expect(fetch).not.toHaveBeenCalled();expect(test).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button',{name:/个人配置.*test-model/}));
  expect(screen.getByLabelText('API Key')).toHaveValue('');expect(screen.getByLabelText('API Key')).toHaveAttribute('placeholder','密钥已保存');
  fireEvent.click(screen.getByRole('button',{name:'取消编辑'}));
  fireEvent.click(screen.getByRole('button',{name:'删除'}));expect(screen.getByRole('alertdialog')).toHaveTextContent('清空');
  fireEvent.click(screen.getByRole('button',{name:'确认删除'}));await screen.findByText('配置已删除。');
  expect((await p.overview()).configs).toHaveLength(0);
 });
 it('retains draft on failed save and guards modal close',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'save').mockRejectedValue(new ModelError('credentialUnavailable'));
  let guard:((action:()=>void)=>void)|null=null;
  await panel(p,value=>{guard=value;});await fill();
  fireEvent.click(screen.getByRole('button',{name:'保存配置'}));await screen.findByRole('alert');
  expect(screen.getByLabelText('API Key')).toHaveValue('dummy-personal-key');
  const close=vi.fn();act(()=>guard!(close));expect(close).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button',{name:'继续编辑'}));expect(screen.getByLabelText('配置名称')).toHaveValue('个人配置');
  act(()=>guard!(close));fireEvent.click(screen.getByRole('button',{name:'放弃修改'}));expect(close).toHaveBeenCalledOnce();
 });
 it('invalidates delayed tests after draft changes and cancels on unmount',async()=>{
  const p=new BrowserModels();let resolve!:(result:ModelTestResult)=>void;
  vi.spyOn(p,'testTextModel').mockImplementation(()=>new Promise(r=>{resolve=r;}));const cancel=vi.spyOn(p,'cancelRequest');
  const {unmount}=render(<ModelPanel port={p}/>);await screen.findByRole('heading',{name:'添加 API 模型'});await fill();
  fireEvent.click(screen.getByRole('button',{name:'测试文字模型'}));
  fireEvent.change(screen.getByLabelText('模型 ID'),{target:{value:'new-model'}});
  await act(async()=>resolve({testedAt:1,elapsedMs:1}));expect(screen.queryByText(/模拟测试完成/)).not.toBeInTheDocument();expect(cancel).toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button',{name:'测试文字模型'}));unmount();expect(cancel).toHaveBeenCalledTimes(2);
 });
 it('password toggles keep keyboard focus, value and draft identity',async()=>{
  await panel();const user=userEvent.setup();const input=screen.getByLabelText('API Key');await user.type(input,'a-secret');
  const toggle=screen.getByRole('button',{name:'显示内容'});await user.click(toggle);
  expect(input).toHaveValue('a-secret');expect(input).toHaveAttribute('type','text');expect(toggle).toHaveFocus();
  await user.click(toggle);expect(input).toHaveAttribute('type','password');expect(toggle).toHaveFocus();
 });
 it('list failure permits manual model entry and saving',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'fetchModels').mockRejectedValue(new ModelError('protocol'));
  await panel(p);await fill();fireEvent.click(screen.getByRole('button',{name:'获取模型列表'}));
  await waitFor(()=>expect(screen.getByText(/服务响应与兼容协议不符/)).toBeInTheDocument());
  fireEvent.click(screen.getByRole('button',{name:'保存配置'}));await screen.findByText('配置已保存。');
 });
});

