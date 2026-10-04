import {createRoot} from 'react-dom/client';
import '../../src/design/tokens.css';
import '../../src/design/base.css';
import {VoiceSurface} from '../../src/features/voice';
createRoot(document.getElementById('root')!).render(<VoiceSurface phase="failed" text={'完整文字供复制。'.repeat(40)} delivery={{state:'unconfirmed',error:'clipboardRestore',restoreError:true,elapsedMs:10}} start={()=>{}} finish={()=>{}} cancel={()=>{}} copy={async()=>{}} restore={async()=>{throw Error('test recovery failure');}}/>);
