import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {ModelPanel} from '../src/features/models';
import {BrowserModels} from '../src/adapters/models-browser';
import {ModelError,type ModelConfigPort,type ModelTestResult} from '../src/contracts/models';
async function panel(port:ModelConfigPort=new BrowserModels(),onGuardChange?:((guard:((action:()=>void)=>void)|null)=>void)){
 render(<ModelPanel port={port} onGuardChange={onGuardChange}/>);
 await screen.findByLabelText('API 密钥');
 return port;
}
function textTab(){fireEvent.mouseDown(screen.getByRole('tab',{name:'文字处理'}),{button:0,ctrlKey:false});}
function fillText(){
 textTab();
 fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'dummy-personal-key'}});
 fireEvent.change(screen.getByLabelText('模型'),{target:{value:'test-model'}});
 fireEvent.click(screen.getByRole('button',{name:'更多设置'}));
 fireEvent.change(screen.getByLabelText('名称'),{target:{value:'个人配置'}});
 fireEvent.change(screen.getByLabelText('服务地址'),{target:{value:'https://example.invalid/v1'}});
}
describe('task-based model settings',()=>{
 it('starts with a speech preset and closes an untouched form immediately',async()=>{
  let guard:((action:()=>void)=>void)|null=null;
  const p=new BrowserModels();await panel(p,value=>{guard=value;});
  expect(screen.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
  expect(screen.queryByLabelText('名称')).not.toBeInTheDocument();
  expect(screen.queryByLabelText('服务地址')).not.toBeInTheDocument();
  const close=vi.fn();act(()=>guard!(close));expect(close).toHaveBeenCalledOnce();
 });
 it('saves a speech key using the preset, without list or test calls',async()=>{
  const p=new BrowserModels();const fetch=vi.spyOn(p,'fetchModels'),test=vi.spyOn(p,'testTextModel');await panel(p);
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'fixture-key'}});
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  const view=await p.overview();expect(view.configs[0]).toMatchObject({name:'通义语音',protocol:'qwen-audio-streaming',baseUrl:'https://dashscope.aliyuncs.com',usages:['speech']});
  expect(view.defaults.find(d=>d.usage==='speech')?.configId).toBe(view.configs[0]!.id);
  expect(fetch).not.toHaveBeenCalled();expect(test).not.toHaveBeenCalled();
 });
 it('saves text separately, keeps the stored key hidden and confirms deletion',async()=>{
  const p=new BrowserModels();await panel(p);fillText();
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).defaults.find(d=>d.usage==='speech')?.configId).toBeNull();
  fireEvent.click(screen.getByRole('button',{name:'编辑'}));
  expect(screen.getByLabelText('API 密钥')).toHaveValue('');expect(screen.getByLabelText('API 密钥')).toHaveAttribute('placeholder','密钥已保存');
  fireEvent.click(screen.getByRole('button',{name:'取消编辑'}));fireEvent.click(screen.getByRole('button',{name:'删除'}));
  const confirm=screen.getByRole('alertdialog');expect(confirm).toHaveTextContent('文字处理将清空');
  fireEvent.click(within(confirm).getByRole('button',{name:'确认删除'}));await screen.findByText('已删除。');expect((await p.overview()).configs).toHaveLength(0);
 });
 it('preserves input on failure and uses a focused visible discard dialog',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'save').mockRejectedValue(new ModelError('credentialUnavailable'));
  let guard:((action:()=>void)=>void)|null=null;await panel(p,value=>{guard=value;});fillText();
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByRole('alert');
  expect(screen.getByLabelText('API 密钥')).toHaveValue('dummy-personal-key');
  const close=vi.fn();act(()=>guard!(close));expect(close).not.toHaveBeenCalled();
  await waitFor(()=>expect(within(screen.getByRole('alertdialog')).getByRole('button',{name:'继续填写'})).toHaveFocus());
  fireEvent.click(screen.getByRole('button',{name:'继续填写'}));expect(screen.getByLabelText('名称')).toHaveValue('个人配置');
  act(()=>guard!(close));fireEvent.click(screen.getByRole('button',{name:'放弃修改'}));expect(close).toHaveBeenCalledOnce();expect(screen.queryByLabelText('API 密钥')).not.toBeInTheDocument();
 });
 it('invalidates delayed tests on draft edits and cancels on unmount',async()=>{
  const p=new BrowserModels();let resolve!:(result:ModelTestResult)=>void;
  vi.spyOn(p,'testTextModel').mockImplementation(()=>new Promise(r=>{resolve=r;}));const cancel=vi.spyOn(p,'cancelRequest');
  const {unmount}=render(<ModelPanel port={p}/>);await screen.findByLabelText('API 密钥');fillText();
  fireEvent.click(screen.getByRole('button',{name:'测试连接'}));fireEvent.change(screen.getByLabelText('模型'),{target:{value:'new-model'}});
  await act(async()=>resolve({testedAt:1,elapsedMs:1}));expect(screen.queryByText('模拟连接成功')).not.toBeInTheDocument();expect(cancel).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button',{name:'测试连接'}));unmount();expect(cancel).toHaveBeenCalledTimes(2);
 });
 it('keeps password value, focus and editor identity while toggling',async()=>{
  await panel();const user=userEvent.setup();const input=screen.getByLabelText('API 密钥');await user.type(input,'a-secret');
  const toggle=screen.getByRole('button',{name:'显示内容'});await user.click(toggle);
  expect(input).toHaveValue('a-secret');expect(input).toHaveAttribute('type','text');expect(toggle).toHaveFocus();
  await user.click(toggle);expect(input).toHaveAttribute('type','password');expect(toggle).toHaveFocus();
 });
 it('allows manual model entry and saving after a list failure',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'fetchModels').mockRejectedValue(new ModelError('protocol'));await panel(p);fillText();
  fireEvent.click(screen.getByRole('button',{name:'获取模型列表'}));await screen.findByText('无法获取有效结果，可以手动填写模型。');
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
 });
 it('guards category switching and keeps both defaults independent',async()=>{
  const p=new BrowserModels();await panel(p);fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'speech-key'}});
  textTab();expect(screen.getByRole('alertdialog')).toBeInTheDocument();fireEvent.click(screen.getByRole('button',{name:'继续填写'}));expect(screen.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  textTab();fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'text-key'}});fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  const view=await p.overview();expect(view.configs).toHaveLength(2);expect(view.defaults[0]!.configId).not.toBe(view.defaults[1]!.configId);
 });
 it('preserves legacy multi-use record usages and custom names during edits',async()=>{
  const p=new BrowserModels();await p.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'原有名称',protocol:'openai-chat',baseUrl:'https://example.invalid/v1',modelId:'legacy-model',usages:['speech','text']}});
  await panel(p);textTab();fireEvent.click(screen.getByRole('button',{name:'编辑'}));
  fireEvent.change(screen.getByLabelText('模型'),{target:{value:'changed-model'}});fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({name:'原有名称',usages:['speech','text'],modelId:'changed-model'});
 });
 it('keeps a legacy speech-only Chat record accessible without promoting its capabilities',async()=>{
  const p=new BrowserModels();await p.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'此前语音配置',protocol:'openai-chat',baseUrl:'https://example.invalid/v1',modelId:'legacy-model',usages:['speech']}});
  await panel(p);textTab();expect(screen.getByText('此前语音配置')).toBeInTheDocument();expect(screen.queryByRole('button',{name:'使用此模型'})).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'编辑'}));fireEvent.change(screen.getByLabelText('模型'),{target:{value:'changed-model'}});fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({name:'此前语音配置',usages:['speech'],modelId:'changed-model'});
 });
});
