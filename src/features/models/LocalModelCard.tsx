import {useEffect,useRef,useState} from 'react';
import {Button,Dialog,InlineMessage,Select} from '../../ui';
import {localAssetId,modelError,modelExpectation,type LocalAssetStatus,type ModelConfigPort,type ModelOverview} from '../../contracts/models';
import {localAsrMessages} from '../../contracts/local-asr';
import {ModelCard} from './ModelCard';
import s from './models.module.css';

export function LocalModelCard({port,view,disabled,mutate,onChanged}:{port:ModelConfigPort;view:ModelOverview;disabled:boolean;mutate:(action:()=>Promise<void>)=>Promise<void>;onChanged:(view:ModelOverview)=>void}){
 const local=port.local;
 const [asset,setAsset]=useState<LocalAssetStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[confirm,setConfirm]=useState(false),[deviceOpen,setDeviceOpen]=useState(false);
 const [device,setDevice]=useState<'auto'|'cpu'>('auto');
 const [selecting,setSelecting]=useState(false);
 const live=useRef(false),flight=useRef(false);
 const records=(view.localConfigs??[]).filter(c=>c.assetId===localAssetId);
 const active=records.find(c=>view.defaults.some(d=>d.usage==='speech'&&d.configId===c.id)),record=active??records[0];
 const selected=Boolean(active),locked=disabled||busy;
 useEffect(()=>{
  if(!local)return;let stopped=false,reading=false;live.current=true;
  const refresh=async()=>{if(reading)return;reading=true;try{const next=await local.status(localAssetId);if(!stopped)setAsset(next);}catch(e){if(!stopped)setError(message(e));}finally{reading=false;}};
  void refresh();const timer=setInterval(()=>void refresh(),1000);
  return()=>{stopped=true;live.current=false;clearInterval(timer);};
 },[local]);
 async function run(action:()=>Promise<void>){
  if(flight.current||disabled)return;flight.current=true;setBusy(true);setError('');
  try{await mutate(action);}catch(e){if(live.current)setError(message(e));}finally{flight.current=false;if(live.current)setBusy(false);}
 }
 async function saveDevice(value:'auto'|'cpu'){
  setDevice(value);if(!record||!local)return;
  await run(async()=>{const next=await local.save({input:{name:record.name,assetId:record.assetId,engine:record.engine,device:value},id:record.id,revision:record.revision});if(live.current)onChanged(next);});
 }
 async function selectLocalModel(){
  if(!local||selected||locked||flight.current)return;
  setSelecting(true);
  try{await run(async()=>{
   const saved=record?view:await local.save({input:{name:'Nemotron 3.5 · 本地流式识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device},id:null,revision:null});
   if(live.current)onChanged(saved);
   const current=record??saved.localConfigs?.find(c=>c.assetId===localAssetId);
   if(!current)throw {code:'unavailable'};
   const choice=saved.defaults.find(d=>d.usage==='speech')!;
   const next=choice.configId===current.id?saved:await port.setDefault({usage:'speech',configId:current.id,revision:choice.revision});
   if(live.current)onChanged(next);
  });}finally{if(live.current)setSelecting(false);}
 }
 async function download(){await run(async()=>{if(local){const next=await local.download(localAssetId);if(live.current)setAsset(next);}});}
 async function remove(){
  if(!local)return;
  await run(async()=>{
   let next=view;
   for(const c of [...records.filter(c=>c!==active),...(active?[active]:[])]){next=await port.remove({id:c.id,revision:c.revision,expectation:modelExpectation(next,c.id,[])});if(live.current)onChanged(next);}
   await local.delete(localAssetId);
   if(live.current){setAsset(await local.status(localAssetId));setConfirm(false);}
  });
 }
 const downloading=asset?.state==='downloading'||asset?.state==='verifying',ready=asset?.state==='ready'||asset?.state==='available';
 const status=!asset?'读取中':asset.state==='verifying'?'校验中':asset.state==='downloading'?`下载 ${Math.floor(asset.received/asset.bytes*100)}%`:asset.state==='failed'?'下载失败':!ready?'未下载':undefined;
 const downloadLabel=asset?.state==='failed'?'重试下载':'下载';
 const replacementView={...view,localConfigs:(view.localConfigs??[]).filter(c=>c.assetId!==localAssetId||c===active)};
 const replacement=active?modelExpectation(replacementView,active.id,[]).replacements.find(r=>r.usage==='speech'):undefined;
 const replacementName=view.configs.find(c=>c.id===replacement?.configId)?.name??view.localConfigs?.find(c=>c.id===replacement?.configId)?.name;
 return <section className={s.localModel} aria-label="本地语音模型">
  <ModelCard name="Nemotron 3.5 · 本地流式识别" subtitle="本地 · 473 MiB" selected={selected} disabled={locked||!ready} pending={selecting} status={status} progress={downloading?{value:asset.received,max:asset.bytes}:undefined} onSelect={()=>void selectLocalModel()}
   action={!ready&&!downloading&&<Button variant="quiet" disabled={locked||!asset} onClick={()=>void download()}>{downloadLabel}</Button>}
   items={ready?[{label:'计算方式',disabled:locked,onSelect:()=>setDeviceOpen(true)},{label:'删除模型',disabled:locked,onSelect:()=>setConfirm(true)}]:downloading?[{label:'取消下载',disabled:locked,onSelect:()=>void run(async()=>{await local?.cancel(localAssetId);})}]:[{label:downloadLabel,disabled:locked||!asset,onSelect:()=>void download()}]}/>
  {asset?.error&&<InlineMessage status="error">{message({code:asset.error})}</InlineMessage>}
  {error&&<InlineMessage status="error">{error}</InlineMessage>}
  {port.mode==='simulation'&&<span className={s.srOnly}>浏览器中的下载及本地识别状态为模拟。</span>}
  <Dialog open={deviceOpen} onOpenChange={open=>{if(!locked)setDeviceOpen(open);}} showTitle title="计算方式" description="选择本地识别的计算方式。" className={s.deviceDialog}>
   <Select label="本地识别计算方式" disabled={locked} value={record?.device??device} onValueChange={value=>void saveDevice(value as 'auto'|'cpu')} options={[{value:'auto',label:'自动选择'},{value:'cpu',label:'仅 CPU'}]}/>
   <Button disabled={locked} onClick={()=>setDeviceOpen(false)}>完成</Button>
  </Dialog>
  <Dialog confirmation open={confirm} onOpenChange={open=>{if(!locked)setConfirm(open);}} title="删除本地模型？" description={selected?`将${replacementName?`改用“${replacementName}”`:'清空语音模型选择'}，然后删除模型文件。历史和词典会保留。`:'将删除模型文件及对应配置。历史和词典会保留。'}>
   <div className={s.actions}><Button disabled={locked} onClick={()=>setConfirm(false)}>取消</Button><Button variant="primary" loading={locked} onClick={()=>void remove()}>确认删除</Button></div>
  </Dialog>
 </section>;
}
function message(error:unknown):string{const code=modelError(error).code;return code in localAsrMessages?localAsrMessages[code as keyof typeof localAsrMessages]:code==='busy'?'模型正在使用或结算，请稍后重试。':code==='conflict'?'模型选择已变化，请重新加载后确认。':'本地模型操作失败，请重试。';}
