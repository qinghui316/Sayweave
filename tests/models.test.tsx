import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {beforeAll,afterAll,describe,it,expect,vi} from 'vitest';
import {ModelPanel} from '../src/features/models';
import {BrowserModels} from '../src/adapters/models-browser';
import {ModelError,type ModelConfigPort,type ModelTestResult} from '../src/contracts/models';
// jsdom does not lay out/scroll elements; real viewport scrolling is checked by Playwright.
const scrollIntoView = HTMLElement.prototype.scrollIntoView;
beforeAll(()=>{HTMLElement.prototype.scrollIntoView=vi.fn();});
afterAll(()=>{if(scrollIntoView) HTMLElement.prototype.scrollIntoView=scrollIntoView; else Reflect.deleteProperty(HTMLElement.prototype,'scrollIntoView');});
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
 fireEvent.change(screen.getByLabelText('服务地址（URL）'),{target:{value:'https://example.invalid/v1'}});
}
describe('task-based model settings',()=>{
 it('maps both services, keeps a same-service draft key and clears it on a service change',async()=>{
  const p=new BrowserModels(),help=vi.fn().mockResolvedValue(undefined);render(<ModelPanel port={p} onHelp={help}/>);await screen.findByLabelText('API 密钥');
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'qwen-fixture-key'}});
  const user=userEvent.setup(),select=screen.getByRole('combobox',{name:'模型'});
  select.focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别'}));
  expect(screen.getByLabelText('API 密钥')).toHaveValue('qwen-fixture-key');
  select.focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('option',{name:'豆包 Seed-ASR 2.0 · 实时识别'}));
  expect(screen.getByLabelText('API 密钥')).toHaveValue('');
  await user.click(screen.getByRole('button',{name:'获取 API 密钥'}));expect(help).toHaveBeenCalledWith('doubao-api-key');
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'doubao-fixture-key'}});
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({protocol:'doubao-asr-streaming',baseUrl:'https://openspeech.bytedance.com',modelId:'Doubao-Seed-ASR-2.0',name:'豆包 Seed-ASR 2.0 · 实时识别',usages:['speech']});
 });
 it('starts with a speech preset and closes an untouched form immediately',async()=>{
  let guard:((action:()=>void)=>void)|null=null;
  const p=new BrowserModels();await panel(p,value=>{guard=value;});
  expect(screen.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
  expect(screen.queryByLabelText('名称')).not.toBeInTheDocument();
  expect(screen.queryByLabelText('服务地址（URL）')).not.toBeInTheDocument();
  const close=vi.fn();act(()=>guard!(close));expect(close).toHaveBeenCalledOnce();
 });
 it('saves a speech key using the preset, without list or test calls',async()=>{
  const p=new BrowserModels();const fetch=vi.spyOn(p,'fetchModels'),test=vi.spyOn(p,'testTextModel');await panel(p);
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'fixture-key'}});
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  const view=await p.overview();expect(view.configs[0]).toMatchObject({name:'Qwen Audio 3.1 · 实时识别',protocol:'qwen-audio-streaming',baseUrl:'https://dashscope.aliyuncs.com',usages:['speech']});
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
 it('allows saving after a connection test fails',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'testTextModel').mockRejectedValue(new ModelError('protocol'));await panel(p);fillText();
  fireEvent.click(screen.getByRole('button',{name:'测试连接'}));await screen.findByText('无法获取有效结果，可以手动填写模型。');
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
 });
 it('guards category switching and keeps both defaults independent',async()=>{
  const p=new BrowserModels();await panel(p);fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'speech-key'}});
  textTab();expect(screen.getByRole('alertdialog')).toBeInTheDocument();fireEvent.click(screen.getByRole('button',{name:'继续填写'}));expect(screen.getByRole('tab',{name:'语音识别'})).toHaveAttribute('aria-selected','true');
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  textTab();fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'text-key'}});fireEvent.change(screen.getByLabelText('模型'),{target:{value:'text-model'}});fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
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
 it('shows only the three required text fields and derives the saved name from the model',async()=>{
  const p=new BrowserModels();await panel(p);textTab();
  expect(screen.getByLabelText('服务地址（URL）')).toHaveValue('https://api.openai.com/v1');
  expect(screen.getByLabelText('API 密钥')).toHaveValue('');expect(screen.getByLabelText('模型')).toHaveValue('');
  expect(screen.getByLabelText('模型')).not.toHaveAttribute('list');
  expect(screen.queryByRole('combobox')).not.toBeInTheDocument();expect(screen.queryByLabelText('名称')).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'fixture-key'}});
  fireEvent.change(screen.getByLabelText('模型'),{target:{value:'gpt-personal'}});
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({name:'gpt-personal',modelId:'gpt-personal',baseUrl:'https://api.openai.com/v1',usages:['text']});
 });
 it('keeps a custom speech model and stored key when explicitly switching to a built-in',async()=>{
  const p=new BrowserModels();await p.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'我的语音',protocol:'qwen-audio-streaming',baseUrl:'https://workspace.example.invalid',modelId:'custom-streaming-id',usages:['speech']}});
  const before=(await p.overview()).configs[0]!;
  render(<ModelPanel port={p}/>);await screen.findByText('我的语音');fireEvent.click(screen.getByRole('button',{name:'编辑'}));
  const select=screen.getByRole('combobox',{name:'模型'});
  expect(select).toHaveTextContent('当前自定义模型 · custom-streaming-id');
  const user=userEvent.setup();select.focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别'}));
  expect(select).toHaveTextContent('Qwen Audio 3.0 · 实时识别');
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({id:before.id,name:'我的语音',baseUrl:before.baseUrl,modelId:'qwen-audio-3.0-asr-flash-streaming',usages:['speech'],credentialStatus:'present',revision:before.revision+1});
 });
 it('saves the selected3.0 model without overwriting it with the initial preset',async()=>{
  const p=new BrowserModels();await panel(p);const user=userEvent.setup();screen.getByRole('combobox',{name:'模型'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别'}));
  fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'fixture-key'}});fireEvent.click(screen.getByRole('button',{name:'保存'}));await screen.findByText('已保存。');
  expect((await p.overview()).configs[0]).toMatchObject({name:'Qwen Audio 3.0 · 实时识别',modelId:'qwen-audio-3.0-asr-flash-streaming'});
 });
 it('focuses an invalid directly visible text model without opening more settings',async()=>{
  const p=new BrowserModels();vi.spyOn(p,'save').mockRejectedValue(new ModelError('invalid','modelId'));await panel(p);textTab();
  fireEvent.click(screen.getByRole('button',{name:'保存'}));await waitFor(()=>expect(screen.getByLabelText('模型')).toHaveFocus());
  expect(screen.queryByLabelText('名称')).not.toBeInTheDocument();
 });

});
