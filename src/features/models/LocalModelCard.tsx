import {useEffect,useRef,useState} from 'react';
import {Button,Dialog,InlineMessage,Select} from '../../ui';
import {localAssetId,modelError,modelExpectation,type LocalAssetStatus,type ModelConfigPort,type ModelOverview} from '../../contracts/models';
import {localAsrMessages} from '../../contracts/local-asr';
import s from './models.module.css';

export function LocalModelCard({port,view,onChanged}:{port:ModelConfigPort;view:ModelOverview;onChanged:(view:ModelOverview)=>void}) {
 const local=port.local;
 const [asset,setAsset]=useState<LocalAssetStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[confirm,setConfirm]=useState(false);
 const [device,setDevice]=useState<'auto'|'cpu'>('auto');
 const live=useRef(false),flight=useRef(false);
 const records=(view.localConfigs??[]).filter(c=>c.assetId===localAssetId);
 const active=records.find(c=>view.defaults.some(d=>d.usage==='speech'&&d.configId===c.id)),record=active??records[0];
 const selected=Boolean(active);
 useEffect(()=>{
  if(!local)return;let stopped=false,reading=false;live.current=true;
  const refresh=async()=>{if(reading)return;reading=true;try {const next=await local.status(localAssetId);if(!stopped)setAsset(next);}catch(e){if(!stopped)setError(message(e));}finally{reading=false;}};
  void refresh();const timer=setInterval(()=>void refresh(),1000);
  return()=>{stopped=true;live.current=false;clearInterval(timer);};
 },[local]);
 async function run(action:()=>Promise<void>){
  if(flight.current)return;flight.current=true;setBusy(true);setError('');
  try {await action();}catch(e){if(live.current)setError(message(e));}finally{flight.current=false;if(live.current)setBusy(false);}
 }
 async function saveDevice(value:'auto'|'cpu') {
  setDevice(value);if(!record||!local)return;
  await run(async()=>{const next=await local.save({input:{name:record.name,assetId:record.assetId,engine:record.engine,device:value},id:record.id,revision:record.revision});if(live.current)onChanged(next);});
 }
 async function selectLocalModel(){
  if(!local)return;
  await run(async()=>{
   const saved=record?view:await local.save({input:{name:'Nemotron 3.5 · 本地流式识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device},id:null,revision:null});
   if(live.current)onChanged(saved);
   const current=record??saved.localConfigs?.find(c=>c.assetId===localAssetId);
   if(!current)throw {code:'unavailable'};
   const choice=saved.defaults.find(d=>d.usage==='speech')!;
   const next=await port.setDefault({usage:'speech',configId:current.id,revision:choice.revision});
   if(live.current)onChanged(next);
  });
 }
 async function remove(){
  if(!local)return;
  await run(async()=>{
   let next=view;
   for(const c of [...records.filter(c=>c!==active),...(active?[active]:[])]){next=await port.remove({id:c.id,revision:c.revision,expectation:modelExpectation(next,c.id,[])});if(live.current)onChanged(next);}
   await local.delete(localAssetId);
   if(live.current){setAsset(await local.status(localAssetId));setConfirm(false);}
  });
 }
 const downloading=asset?.state==='downloading'||asset?.state==='verifying';
 const ready=asset?.state==='ready'||asset?.state==='available';
 const replacementView={...view,localConfigs:(view.localConfigs??[]).filter(c=>c.assetId!==localAssetId||c===active)};
 const replacement=active?modelExpectation(replacementView,active.id,[]).replacements.find(r=>r.usage==='speech'):undefined;
 const replacementName=view.configs.find(c=>c.id===replacement?.configId)?.name??view.localConfigs?.find(c=>c.id===replacement?.configId)?.name;
 return <section className={s.localModel} aria-label="本地语音模型">
  <div className={s.name}><strong>Nemotron 3.5 · 本地流式识别</strong>{selected&&<span className={s.current}>当前使用</span>}</div>
  <p className={s.meta}>模型约 473 MiB。识别在本机完成；开启文字整理时，文字仍会发送至所选文字模型。</p>
  {port.mode==='simulation'&&<p className={s.meta}>浏览器中的下载及本地识别状态为模拟。</p>}
  <div className={s.localOptions}><span>计算方式</span><Select label="本地识别计算方式" disabled={busy} value={record?.device??device} onValueChange={value=>void saveDevice(value as 'auto'|'cpu')} options={[{value:'auto',label:'自动选择'},{value:'cpu',label:'仅 CPU'}]}/></div>
  {downloading&&<div role="status"><p className={s.meta}>{asset.state==='verifying'?'正在校验模型…':`正在下载 ${Math.floor(asset.received/asset.bytes*100)}%`}</p><progress aria-label="模型下载进度" max={asset.bytes} value={asset.received}/></div>}
  {asset?.error&&<InlineMessage status="error">{message({code:asset.error})}</InlineMessage>}
  {error&&<InlineMessage status="error">{error}</InlineMessage>}
  <div className={s.actions}>
   {!ready&&!downloading&&<Button disabled={busy||!asset} onClick={()=>void run(async()=>{if(local){const next=await local.download(localAssetId);if(live.current)setAsset(next);}})}>{asset?.state==='failed'?'重试下载':'下载模型'}</Button>}
   {downloading&&<Button disabled={busy} onClick={()=>void run(async()=>{await local?.cancel(localAssetId);})}>取消下载</Button>}
   {ready&&!selected&&<Button disabled={busy} onClick={()=>void selectLocalModel()}>使用本地模型</Button>}
   {ready&&<Button variant="quiet" disabled={busy} onClick={()=>setConfirm(true)}>删除本地模型</Button>}
  </div>
  <Dialog confirmation open={confirm} onOpenChange={open=>{if(!busy)setConfirm(open);}} title="删除本地模型？" description={selected?`将${replacementName?`改用“${replacementName}”`:'清空语音模型选择'}，然后删除模型文件。历史和词典会保留。`:'将删除模型文件及对应配置。历史和词典会保留。'}>
   <div className={s.actions}><Button disabled={busy} onClick={()=>setConfirm(false)}>取消</Button><Button variant="primary" loading={busy} onClick={()=>void remove()}>确认删除</Button></div>
  </Dialog>
 </section>;
}
function message(error:unknown):string {const code=modelError(error).code;return code in localAsrMessages?localAsrMessages[code as keyof typeof localAsrMessages]:code==='busy'?'模型正在使用或结算，请稍后重试。':code==='conflict'?'模型选择已变化，请重新加载后确认。':'本地模型操作失败，请重试。';}
