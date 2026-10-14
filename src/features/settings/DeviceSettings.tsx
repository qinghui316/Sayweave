import {useEffect,useState,useRef,useId} from 'react';
import {Button,InlineMessage,Select,SettingsRow,SettingsSection} from '../../ui';
import {speechError,type Microphone,type RecordingPreferences,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
export function DeviceSettings({port,settings}:{port:SpeechPort;settings:SettingStore<RecordingPreferences>}){
 const [devices,setDevices]=useState<Microphone[]>([]),[error,setError]=useState(''),[refresh,setRefresh]=useState(0);
 const generation=useRef(0),state=useSetting(settings);
 useEffect(()=>{const token=++generation.current;let alive=true;void port.devices().then(d=>{if(alive&&token===generation.current){setDevices(d);setError('');}}).catch(e=>{if(alive&&token===generation.current)setError(speechMessages[speechError(e).code]);});return()=>{alive=false;};},[port,refresh]);
 const prefs=state.value,id=useId();
 return <SettingsSection title="音频输入"><SettingsRow title={<label htmlFor={id}>录音设备</label>} description="更换后，下一次录音生效。" feedback={(error||state.error)&&<InlineMessage status="error">{error||state.error}</InlineMessage>}>
  <Select id={id} label="录音设备" disabled={!state.ready} value={prefs?.deviceId??'system-default'} onValueChange={id=>settings.update({deviceId:id==='system-default'?null:id})} options={[{value:'system-default',label:'系统默认麦克风'},...(prefs?.deviceId&&!devices.some(d=>d.id===prefs.deviceId)?[{value:prefs.deviceId,label:'已选设备不可用'}]:[]),...devices.map(d=>({value:d.id,label:d.name+(d.isDefault?' · 系统默认':'')}))]}/>
  <Button variant="quiet" onClick={()=>{setRefresh(v=>v+1);void settings.load();}}>刷新设备</Button>
 </SettingsRow></SettingsSection>;
}
