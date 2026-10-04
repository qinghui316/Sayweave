import {useEffect,useState,useRef} from 'react';
import {Button,Field,InlineMessage} from '../../ui';
import {speechError,type Microphone,type RecordingPreferences,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
export function DeviceSettings({port}:{port:SpeechPort}){
 const [devices,setDevices]=useState<Microphone[]>([]),[prefs,setPrefs]=useState<RecordingPreferences|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false),[refresh,setRefresh]=useState(0);
 const generation=useRef(0),mounted=useRef(false),saving=useRef(false);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 useEffect(()=>{const token=++generation.current;let alive=true;Promise.all([port.devices(),port.preferences()]).then(([d,p])=>{if(alive&&token===generation.current){setDevices(d);setPrefs(previous=>previous&&previous.revision>p.revision?previous:p);setError('');}}).catch(e=>{if(alive&&token===generation.current)setError(speechMessages[speechError(e).code]);});return()=>{alive=false;};},[port,refresh]);
 async function save(id:string){if(!prefs||saving.current)return;const token=++generation.current;saving.current=true;setBusy(true);try{const p=await port.savePreferences({...prefs,deviceId:id||null});if(mounted.current&&token===generation.current){setPrefs(p);setError('');}}catch(e){if(mounted.current&&token===generation.current)setError(speechMessages[speechError(e).code]);}finally{saving.current=false;if(mounted.current&&token===generation.current)setBusy(false);}}
 return <section><h3>麦克风</h3><p>更换后，下一次录音生效。</p>{port.mode==='simulation'&&<p>浏览器模拟，不访问设备。</p>}{prefs&&<Field label="录音设备">{p=><select {...p} disabled={busy} value={prefs.deviceId??''} onChange={e=>void save(e.target.value)}><option value="">系统默认麦克风</option>{prefs.deviceId&&!devices.some(d=>d.id===prefs.deviceId)&&<option value={prefs.deviceId}>已选设备不可用</option>}{devices.map(d=><option key={d.id} value={d.id}>{d.name}{d.isDefault?' · 系统默认':''}</option>)}</select>}</Field>}{error&&<InlineMessage status="error">{error}</InlineMessage>}<Button disabled={busy} onClick={()=>setRefresh(v=>v+1)}>刷新设备</Button></section>;
}
