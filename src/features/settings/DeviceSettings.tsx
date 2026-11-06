import {useEffect,useState,useRef,useId,type CSSProperties} from 'react';
import {Button,InlineMessage,Select,SettingsRow,SettingsSection,Switch} from '../../ui';
import {INPUT_GAIN_STOPS,isInputGainDb,speechError,type InputGainDb,type Microphone,type RecordingPreferences,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import s from './settings.module.css';

const gainMax=INPUT_GAIN_STOPS[4];
const gainLabels={0:'Normal',3:'Low',6:'Medium',9:'High',12:'Max'} satisfies Record<InputGainDb,string>;
const gainPixels=[[9,28],[18,65],[30,34],[39,72],[45,19],[53,51],[60,32],[65,77],[71,15],[74,56],[80,34],[83,72],[88,22],[92,53]] as const;
const gainRays=[[-31,-8],[-21,-20],[-8,-26],[7,-24],[20,-18],[25,-5],[21,15],[7,23],[-13,23],[-27,13]] as const;

export function DeviceSettings({port,settings}:{port:SpeechPort;settings:SettingStore<RecordingPreferences>}){
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
 const gain=prefs?.inputGainDb??0,gainLabel=gainLabels[gain];
 const isMax=gain===gainMax;
 const bursting=isMax&&maxBurst;
 return <SettingsSection title="音频输入"><SettingsRow title={<label htmlFor={id}>录音设备</label>} description="更换后，下一次录音生效。" feedback={(error||state.error)&&<InlineMessage status="error">{error||state.error}</InlineMessage>}>
  <Select id={id} label="录音设备" disabled={!state.ready} value={prefs?.deviceId??'system-default'} onValueChange={id=>settings.update({deviceId:id==='system-default'?null:id})} options={[{value:'system-default',label:'系统默认麦克风'},...(prefs?.deviceId&&!devices.some(d=>d.id===prefs.deviceId)?[{value:prefs.deviceId,label:'已选设备不可用'}]:[]),...devices.map(d=>({value:d.id,label:d.name+(d.isDefault?' · 系统默认':'')}))]}/>
  <Button variant="quiet" onClick={()=>{setRefresh(v=>v+1);void settings.load();}}>刷新设备</Button>
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
