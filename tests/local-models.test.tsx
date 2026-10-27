import userEvent from '@testing-library/user-event';
import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import {describe,it,expect,vi} from 'vitest';
import {BrowserModels} from '../src/adapters/models-browser';
import {ModelPanel} from '../src/features/models';
import {localAssetId,parseModelOverview,modelExpectation} from '../src/contracts/models';

describe('local model contracts and settings',()=>{
 it('confirms the actual local replacement before deleting the current cloud default',async()=>{
  const port=new BrowserModels();
  await port.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'云端识别',protocol:'qwen-audio-streaming',baseUrl:'https://example.com',modelId:'fixture',usages:['speech']}});
  await port.local.download();
  await port.local.save({input:{name:'备用本地识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'cpu'},id:null,revision:null});
  render(<ModelPanel port={port}/>);
  await modelAction('删除');
  const dialog=screen.getByRole('alertdialog');
  expect(dialog).toHaveTextContent('备用本地识别');expect(dialog).not.toHaveTextContent('清空');
  fireEvent.click(within(dialog).getByRole('button',{name:'确认删除'}));
  await waitFor(async()=>expect((await port.overview()).configs).toEqual([]));
  const view=await port.overview();
  expect(view.defaults.find(d=>d.usage==='speech')!.configId).toBe(view.localConfigs![0]!.id);
 });
 it('download does not change defaults; use is explicit and device remains frozen in saved config',async()=>{
  const port=new BrowserModels();render(<ModelPanel port={port}/>);
  const card=await screen.findByRole('region',{name:'本地语音模型'});
  fireEvent.click(await within(card).findByRole('button',{name:'下载'}));
  expect((await port.overview()).defaults.every(d=>d.configId===null)).toBe(true);
  fireEvent.click(await within(card).findByRole('button',{name:'Nemotron 3.5 · 本地流式识别'}));
  await waitFor(()=>expect(within(card).getByRole('button',{name:'Nemotron 3.5 · 本地流式识别'})).toHaveAttribute('aria-pressed','true'));
  const view=await port.overview();const record=view.localConfigs![0]!;
  expect(record).toMatchObject({assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'auto'});
  expect(view.configs).toEqual([]);expect(view.defaults.find(d=>d.usage==='speech')?.configId).toBe(record.id);
  expect(JSON.stringify(record)).not.toMatch(/baseUrl|credential|key/u);
 });
 it('deleting the selected local config follows replacement confirmation and preserves cloud configuration',async()=>{
  const port=new BrowserModels();await port.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'云端识别',protocol:'qwen-audio-streaming',baseUrl:'https://example.com',modelId:'fixture',usages:['speech']}});
  await port.local.download();let view=await port.local.save({input:{name:'本地识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'cpu'},id:null,revision:null});
  view=await port.setDefault({usage:'speech',configId:view.localConfigs![0]!.id,revision:view.defaults.find(d=>d.usage==='speech')!.revision});
  render(<ModelPanel port={port}/>);
  await modelAction('删除模型',true);
  const dialog=screen.getByRole('alertdialog');expect(dialog).toHaveTextContent('云端识别');
  fireEvent.click(within(dialog).getByRole('button',{name:'确认删除'}));
  await waitFor(async()=>expect((await port.overview()).localConfigs).toEqual([]));
  expect((await port.overview()).configs).toHaveLength(1);
  expect((await port.overview()).defaults.find(d=>d.usage==='speech')!.configId).toBe(view.configs[0]!.id);
 });
 it('rejects local-as-text defaults and duplicate identity while accepting existing cloud-only fixtures',async()=>{
  const port=new BrowserModels();const before=await port.overview();expect(parseModelOverview(before).localConfigs).toEqual([]);
  const view=await port.local.save({input:{name:'local',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'auto'},id:null,revision:null});
  const c=view.localConfigs![0]!;
  expect(()=>parseModelOverview({...view,defaults:view.defaults.map(d=>d.usage==='text'?{...d,configId:c.id}:d)})).toThrow();
  expect(()=>parseModelOverview({...view,localConfigs:[c,c]})).toThrow();
  expect(modelExpectation(view,c.id,[]).replacements).toEqual([]);
 });
 it('edits the active local device and removes inactive duplicates before replacing the default',async()=>{
  const port=new BrowserModels();await port.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'云端识别',protocol:'qwen-audio-streaming',baseUrl:'https://example.com',modelId:'fixture',usages:['speech']}});
  await port.local.download();
  const input={name:'本地识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'auto' as const};
  await port.local.save({input,id:null,revision:null});
  const view=await port.local.save({input:{...input,name:'当前本地模型',device:'cpu'},id:null,revision:null});
  const current=view.localConfigs!.find(c=>c.name==='当前本地模型')!;
  await port.setDefault({usage:'speech',configId:current.id,revision:view.defaults.find(d=>d.usage==='speech')!.revision});
  render(<ModelPanel port={port}/>);
  await screen.findByRole('region',{name:'本地语音模型'});
  await modelAction('计算方式',true);
  expect(screen.getByRole('combobox',{name:'本地识别计算方式'})).toHaveTextContent('仅 CPU');
  fireEvent.click(screen.getByRole('button',{name:'完成'}));
  await modelAction('删除模型',true);
  const dialog=screen.getByRole('alertdialog');expect(dialog).toHaveTextContent('云端识别');expect(dialog).not.toHaveTextContent('当前本地模型');
  fireEvent.click(within(dialog).getByRole('button',{name:'确认删除'}));
  await waitFor(async()=>expect((await port.overview()).localConfigs).toEqual([]));
  expect((await port.overview()).defaults.find(d=>d.usage==='speech')!.configId).toBe(view.configs[0]!.id);
 });
 it('ignores a status response after the settings card unmounts',async()=>{
  const port=new BrowserModels();let resolve!:(v:Awaited<ReturnType<typeof port.local.status>>)=>void;
  const pending=new Promise<Awaited<ReturnType<typeof port.local.status>>>(r=>{resolve=r;});
  vi.spyOn(port.local,'status').mockReturnValue(pending);
  const view=render(<ModelPanel port={port}/>);await screen.findByText('Nemotron 3.5 · 本地流式识别');view.unmount();
  await act(async()=>resolve({assetId:localAssetId,state:'ready',bytes:495831520,received:495831520,error:null}));
 });
});

async function modelAction(label:string,local=false){
 const root=local?within(await screen.findByRole('region',{name:'本地语音模型'})):screen;
 await userEvent.click((await root.findAllByRole('button',{name:/^更多操作 /u}))[0]!);
 await userEvent.click(await screen.findByRole('menuitem',{name:label}));
}

describe('local card pending feedback',()=>{
 it('waits for the accepted local default in the reserved slot and keeps cloud selection on failure',async()=>{
  const port=new BrowserModels();await port.save({id:null,revision:null,expectation:null,key:{action:'replace',value:'fixture-key'},input:{name:'云端基线',protocol:'qwen-audio-streaming',baseUrl:'https://example.com',modelId:'fixture',usages:['speech']}});await port.local.download();
  let reject!:(error:unknown)=>void;vi.spyOn(port,'setDefault').mockImplementation(()=>new Promise((_resolve,no)=>{reject=no;}));
  render(<ModelPanel port={port}/>);const cloud=await screen.findByRole('button',{name:'云端基线'}),local=screen.getByRole('button',{name:'Nemotron 3.5 · 本地流式识别'});await waitFor(()=>expect(local).toBeEnabled());
  await userEvent.click(local);await waitFor(()=>expect(port.setDefault).toHaveBeenCalledOnce());
  expect(local).toHaveAttribute('aria-busy','true');expect(local).toHaveAttribute('aria-pressed','false');expect(cloud).toHaveAttribute('aria-pressed','true');
  await act(async()=>reject({code:'conflict'}));await screen.findByRole('alert');expect(local).not.toHaveAttribute('aria-busy');expect(local).toBeEnabled();expect(cloud).toHaveAttribute('aria-pressed','true');
  expect(screen.queryByText('本机识别；文字整理按所选文字模型处理。')).not.toBeInTheDocument();
 });
});
