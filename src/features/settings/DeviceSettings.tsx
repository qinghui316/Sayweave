import {useEffect,useState,useRef,useId,type CSSProperties} from 'react';
import {Button,InlineMessage,Select,SettingsRow,SettingsSection,Switch} from '../../ui';
import {INPUT_GAIN_STOPS,isInputGainDb,speechError,type InputGainDb,type Microphone,type RecordingPreferences,type SpeechErrorCode,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import s from './settings.module.css';

const gainMax=INPUT_GAIN_STOPS[4];
const gainLabels={0:'Normal',3:'Low',6:'Medium',9:'High',12:'Max'} satisfies Record<InputGainDb,string>;
const gainPixels=[[9,28],[18,65],[30,34],[39,72],[45,19],[53,51],[60,32],[65,77],[71,15],[74,56],[80,34],[83,72],[88,22],[92,53]] as const;
const gainRays=[[-31,-8],[-21,-20],[-8,-26],[7,-24],[20,-18],[25,-5],[21,15],[7,23],[-13,23],[-27,13]] as const;

export function DeviceSettings({port,settings,microphoneTestAvailable=true}:{port:SpeechPort;settings:SettingStore<RecordingPreferences>;microphoneTestAvailable?:boolean}){
 const [devices,setDevices]=useState<Microphone[]>([]),[error,setError]=useState(''),[refresh,setRefresh]=useState(0);
 const [maxBurst,setMaxBurst]=useState(false);
 const generation=useRef(0),state=useSetting(settings);
 useEffect(()=>{const token=++generation.current;let alive=true;void port.devices().then(d=>{if(alive&&token===generation.current){setDevices(d);setError('');}}).catch(e=>{if(alive&&token===generation.current)setError(speechMessages[speechError(e).code]);});return()=>{alive=false;};},[port,refresh]);
 useEffect(()=>{
  if(!maxBurst)return;
  const timer=window.setTimeout(()=>setMaxBurst(false),780);
  return()=>window.clearTimeout(timer);
 },[maxBurst]);
 const prefs=state.value,id=useId(),gainId=useId(),gainHelpId=useId(),soundsId=useId();
 const confirmed=settings.getConfirmed();
 const defaultDevice=devices.find(device=>device.isDefault);
 const gain=prefs?.inputGainDb??0,gainLabel=gainLabels[gain];
 const isMax=gain===gainMax;
 const bursting=isMax&&maxBurst;
 return <SettingsSection title="音频输入"><SettingsRow title={<label htmlFor={id}>录音设备</label>} description={<>更换后，下一次录音生效。 <button type="button" className={s.refreshDevices} onClick={()=>{setRefresh(v=>v+1);void settings.load();}}>刷新设备</button></>} feedback={(error||state.error)&&<InlineMessage status="error">{error||state.error}</InlineMessage>}>
  <Select id={id} label="录音设备" disabled={!state.ready} value={prefs?.deviceId??'system-default'} onValueChange={id=>settings.update({deviceId:id==='system-default'?null:id})} options={[{value:'system-default',label:defaultDevice?`系统默认 · ${defaultDevice.name}`:'系统默认麦克风'},...(prefs?.deviceId&&!devices.some(d=>d.id===prefs.deviceId)?[{value:prefs.deviceId,label:'已选设备不可用'}]:[]),...devices.map(d=>({value:d.id,label:d.name+(d.isDefault?' · 系统默认':'')}))]}/>
 </SettingsRow>
 <SettingsRow title={<label htmlFor={gainId}>麦克风增益</label>} description={<span id={gainHelpId}>调整麦克风增益。说悄悄话也能听见。</span>}>
  <div className={s.inputGain}>
   <div className={s.inputGainControl} data-gain-max={isMax||undefined} data-gain-burst={bursting||undefined} style={{'--gain-ratio':gain/gainMax} as CSSProperties}>
    <div className={s.inputGainTrack} aria-hidden="true">
     <div className={s.inputGainFill}/>
     {isMax&&gainPixels.map(([x,y],i)=><span key={x} className={s.inputGainPixel} style={{left:`${x}%`,top:`${y}%`,'--gain-delay':`${(i%5)*44}ms`} as CSSProperties}/>)}
     {INPUT_GAIN_STOPS.map(stop=><span key={stop} className={s.inputGainStop} data-filled={stop<=gain||undefined} style={{'--gain-stop':stop/gainMax} as CSSProperties}/>)}
    </div>
    {bursting&&<div className={s.inputGainBurst} aria-hidden="true">
     {gainRays.map(([x,y],i)=><span key={`${x}/${y}`} className={s.inputGainRay} style={{'--gain-ray-x':`${x}px`,'--gain-ray-y':`${y}px`,'--gain-delay':`${(i%3)*24}ms`} as CSSProperties}/>)}
    </div>}
    <div className={s.inputGainThumb} aria-hidden="true"/>
    <input className={s.inputGainRange} id={gainId} type="range" min={INPUT_GAIN_STOPS[0]} max={gainMax} step={3} value={gain} aria-describedby={gainHelpId} aria-valuetext={gainLabel} disabled={!state.ready} onChange={event=>{
     const inputGainDb=event.currentTarget.valueAsNumber;
     if(isInputGainDb(inputGainDb)){
      setMaxBurst(inputGainDb===gainMax&&!isMax);
      settings.update({inputGainDb});
     }
    }}/>
   </div>
   <span className={s.inputGainValue} aria-hidden="true">{prefs?gainLabel:'—'}</span>
  </div>
  <MicrophoneTest key={microphoneTestAvailable?'available':'paused'} port={port} settings={settings} deviceId={confirmed?.deviceId} gainDb={confirmed?.inputGainDb??0} refresh={refresh} available={microphoneTestAvailable&&state.ready}/>
 </SettingsRow>
 <SettingsRow title={<label htmlFor={soundsId}>交互声音</label>} description="录音就绪、最终结果可用时播放提示音。">
  <Switch
   id={soundsId}
   label="交互声音"
   disabled={!state.ready}
   checked={prefs?.interactionSoundsEnabled??false}
   onCheckedChange={interactionSoundsEnabled=>settings.update({interactionSoundsEnabled})}
  />
 </SettingsRow>
 </SettingsSection>;
}

function MicrophoneTest({port,settings,deviceId,gainDb,refresh,available}:{
 port:SpeechPort;settings:SettingStore<RecordingPreferences>;deviceId:string|null|undefined;gainDb:InputGainDb;refresh:number;available:boolean
}){
 const [requested,setRequested]=useState(false);
 const [reading,setReading]=useState<{deviceId:string|null;refresh:number;level:number}|null>(null);
 const [failure,setFailure]=useState<SpeechErrorCode|null>(null);
 const [settingsError,setSettingsError]=useState(false);
 const active=useRef<{id:string;gainDb:InputGainDb}|null>(null);
 useEffect(()=>{
  if(!requested||!available||deviceId===undefined)return;
  const id=crypto.randomUUID(),gainDb=settings.getConfirmed()?.inputGainDb??0;
  active.current={id,gainDb};
  const finish=(error:SpeechErrorCode|null)=>{
   if(active.current?.id!==id)return;
   setRequested(false);
   setReading(null);
   setFailure(error);
  };
  void port.startMicrophoneTest({id,deviceId,gainDb},event=>{
   if(active.current?.id!==id)return;
   if(event.kind==='level')setReading({deviceId,refresh,level:event.level});
   else finish(event.error);
  }).catch(error=>finish(speechError(error).code));
  return ()=>{
   active.current=null;
   void port.controlMicrophoneTest({action:'stop',id}).catch(()=>{});
  };
 },[available,deviceId,port,refresh,requested,settings]);
 useEffect(()=>{
  const test=active.current;
  if(!test||test.gainDb===gainDb)return;
  test.gainDb=gainDb;
  void port.controlMicrophoneTest({action:'gain',id:test.id,gainDb}).catch(error=>{
   if(active.current!==test)return;
   setRequested(false);
   setReading(null);
   setFailure(speechError(error).code);
  });
 },[gainDb,port]);
 const current=requested&&available&&reading?.deviceId===deviceId&&reading?.refresh===refresh?reading:null;
 const level=current?.level??0;
 // Fixed -60 to 0 dBFS scale keeps a gain change visible.
 const meter=level>0?Math.max(0,Math.min(1,(20*Math.log10(level)+60)/60)):0;
 const deviceFailure=failure==='deviceMissing'||failure==='deviceAccess'||failure==='deviceFormat';
 const message=failure==='deviceMissing'?(deviceId?'所选麦克风已断开，请重新选择。':'未找到系统默认麦克风。'):failure?speechMessages[failure]:null;
 return <>
  <div className={s.microphoneTest}>
   <meter aria-label="输入音量" aria-busy={requested&&current===null} title={port.mode==='simulation'?'模拟输入音量':'说句话，查看输入音量。'} min={0} max={1} value={meter}/>
   <Button variant="quiet" disabled={!available} onClick={()=>{
    setReading(null);
    setFailure(null);
    setSettingsError(false);
    setRequested(!requested);
   }}>{requested?'停止测试':'测试麦克风'}</Button>
  </div>
  {message&&<InlineMessage status="error">{message}</InlineMessage>}
  {deviceFailure&&port.mode==='native'&&<Button variant="quiet" onClick={()=>{
   setSettingsError(false);
   void port.openMicrophoneSettings().catch(()=>setSettingsError(true));
  }}>打开 Windows 麦克风设置</Button>}
  {settingsError&&<InlineMessage status="error">无法打开 Windows 麦克风设置。</InlineMessage>}
 </>;
}
