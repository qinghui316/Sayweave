import {useState} from 'react';
import {createRoot} from 'react-dom/client';
import '../../src/design/tokens.css';
import '../../src/design/base.css';
import {VoiceSurface} from '../../src/features/voice';
import type {DictationPresentation} from '../../src/contracts/dictation';
function Fixture(){
 const kind=(new URLSearchParams(location.search).get('presentation')??'notice') as DictationPresentation;
 const [dismissed,setDismissed]=useState(false),[copied,setCopied]=useState(false);
 if(dismissed)return <p role="status">已关闭</p>;
 return <VoiceSurface presentation={kind} phase={kind==='recording'?'recording':kind==='completed'?'completed':'processing'} level={.05} text={'完整文字供复制。'.repeat(40)} delivery={kind==='notice'?{state:'unconfirmed',error:'inputUnconfirmed',restoreError:true,elapsedMs:10}:null} start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={()=>setDismissed(true)} copy={async()=>{if(!copied){setCopied(true);throw Error('test copy failure');}}} restore={async()=>{throw Error('test recovery failure');}}/>;
}
createRoot(document.getElementById('root')!).render(<Fixture/>);
