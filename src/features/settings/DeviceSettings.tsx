import {useEffect,useState,useRef} from 'react';
import {Button,Field,InlineMessage,Select} from '../../ui';
import s from './settings.module.css';
import {speechError,type Microphone,type RecordingPreferences,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
export function DeviceSettings({port,settings}:{port:SpeechPort;settings:SettingStore<RecordingPreferences>}){
 const [devices,setDevices]=useState<Microphone[]>([]),[error,setError]=useState(''),[refresh,setRefresh]=useState(0);
 const generation=useRef(0),state=useSetting(settings);
 useEffect(()=>{const token=++generation.current;let alive=true;void port.devices().then(d=>{if(alive&&token===generation.current){setDevices(d);setError('');}}).catch(e=>{if(alive&&token===generation.current)setError(speechMessages[speechError(e).code]);});return()=>{alive=false;};},[port,refresh]);
 const prefs=state.value;
 return <section className={s.deviceSection}><h3>麦克风</h3><div className={s.deviceRow}><Field label="录音设备">{p=><Select {...p} label="录音设备" disabled={!state.ready} value={prefs?.deviceId??'system-default'} onValueChange={id=>settings.update({deviceId:id==='system-default'?null:id})} options={[{value:'system-default',label:'系统默认麦克风'},...(prefs?.deviceId&&!devices.some(d=>d.id===prefs.deviceId)?[{value:prefs.deviceId,label:'已选设备不可用'}]:[]),...devices.map(d=>({value:d.id,label:d.name+(d.isDefault?' · 系统默认':'')}))]}/>}</Field><Button onClick={()=>{setRefresh(v=>v+1);void settings.load();}}>刷新设备</Button></div>{(error||state.error)&&<InlineMessage status="error">{error||state.error}</InlineMessage>}</section>;
}
