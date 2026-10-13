import {useState,useEffect} from 'react';
import {createRoot} from 'react-dom/client';
import '../../src/design/tokens.css';
import '../../src/design/base.css';
import {VoiceSurface} from '../../src/features/voice';
import type {DictationPresentation} from '../../src/contracts/dictation';
function Fixture(){
 const [kind,setKind]=useState((new URLSearchParams(location.search).get('presentation')??'notice') as DictationPresentation);
 useEffect(()=>{const change=(event:Event)=>setKind((event as CustomEvent<DictationPresentation>).detail);window.addEventListener('fixture:presentation',change);return()=>window.removeEventListener('fixture:presentation',change);},[]);
 const [dismissed,setDismissed]=useState(false),[copied,setCopied]=useState(false);
 if(dismissed)return <p role="status">已关闭</p>;
 return <VoiceSurface presentation={kind} phase={kind==='recording'?'recording':kind==='completed'?'completed':'processing'} level={.05} error={kind==='error'?'输入组件连接中断，请重新开始口述。':undefined} configure={kind==='error'?()=>document.documentElement.setAttribute('data-configured','true'):undefined} text={'完整文字供复制。'.repeat(40)} delivery={kind==='notice'?{state:'unconfirmed',submission:'submitted',verification:'timeout',stage:'verify',submittedEvents:4,error:'inputUnconfirmed',restoreError:true,elapsedMs:10}:null} start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={()=>setDismissed(true)} copy={async()=>{if(!copied){setCopied(true);throw Error('test copy failure');}}}/>;
}
createRoot(document.getElementById('root')!).render(<Fixture/>);
