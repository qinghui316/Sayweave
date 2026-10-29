import {applyTheme} from '../../src/design/themes';
import {Button,Dialog,Icon,Select,ActionMenu} from '../../src/ui';
import {ModelPicker} from '../../src/features/models/ModelPicker';
import {useState,useEffect} from 'react';
import {createRoot} from 'react-dom/client';
import '../../src/design/tokens.css';
import '../../src/design/base.css';
import {BrowserTranslation} from '../../src/adapters/translation-tauri';
import {initialDictation,parseDictation} from '../../src/contracts/dictation';
import {VoiceSurface} from '../../src/features/voice';
import type {DictationPresentation} from '../../src/contracts/dictation';
// This isolated entry bypasses main.tsx; use the same catalogue before rendering.
applyTheme('light');
function Fixture(){
 const [kind,setKind]=useState((new URLSearchParams(location.search).get('presentation')??'notice') as DictationPresentation);
 useEffect(()=>{const change=(event:Event)=>setKind((event as CustomEvent<DictationPresentation>).detail);window.addEventListener('fixture:presentation',change);return()=>window.removeEventListener('fixture:presentation',change);},[]);
 const [dismissed,setDismissed]=useState(false),[copied,setCopied]=useState(false);
 if(dismissed)return <p role="status">已关闭</p>;
 return <VoiceSurface presentation={kind} phase={kind==='recording'?'recording':kind==='completed'?'completed':'processing'} level={.05} error={kind==='error'?'输入组件连接中断，请重新开始口述。':undefined} configure={kind==='error'?()=>document.documentElement.setAttribute('data-configured','true'):undefined} text={'完整文字供复制。'.repeat(40)} delivery={kind==='notice'?{state:'unconfirmed',submission:'submitted',verification:'timeout',stage:'verify',submittedEvents:4,error:'inputUnconfirmed',restoreError:true,elapsedMs:10}:null} start={()=>{}} finish={()=>{}} cancel={()=>{}} dismiss={()=>setDismissed(true)} copy={async()=>{if(!copied){setCopied(true);throw Error('test copy failure');}}}/>;
}
createRoot(document.getElementById('root')!).render(new URLSearchParams(location.search).get('surface')==='popups'?<PopupFixture/>:new URLSearchParams(location.search).get('surface')==='translation'?<TranslationFixture/>:<Fixture/>);

function PopupFixture(){
 const [open,setOpen]=useState(false),[choice,setChoice]=useState('a'),[model,setModel]=useState('model-a');
 return <div style={{padding:32,color:'white',background:'black'}}>
  <Button onClick={()=>setOpen(true)}>打开确认框</Button>
  <Dialog confirmation open={open} onOpenChange={setOpen} title="保存修改" description="保存后继续。">
   <Button variant="primary" onClick={()=>setOpen(false)}><Icon name="square-pen"/>保存</Button>
   <Button disabled>处理中</Button><Button onClick={()=>setOpen(false)}>取消</Button>
  </Dialog>
  <Select value={choice} onValueChange={setChoice} label="测试选项" options={[{value:'a',label:'选项甲'},{value:'b',label:'选项乙'}]}/>
  <ActionMenu label="打开操作菜单" items={[{label:'编辑',onSelect:()=>{}},{label:'暂不可用',disabled:true,onSelect:()=>{}}]}/>
  <ModelPicker id="fixture-model" aria-invalid={false} value={model} ids={['model-a','model-b']} loading={false} error={null} onOpen={()=>{}} onRefresh={()=>{}} onChange={setModel}/>
 </div>;
}

function TranslationFixture(){
 const [snapshot,setSnapshot]=useState(parseDictation({...initialDictation(),purpose:'translation' as const,taskId:'translation-fixture',presentationId:'translation-view',presentation:'recording' as const,targetLanguage:'en-US',translationTargets:['en-US','ja','zh-Hant','fr','de'],translationSelectable:true}));
 const [base]=useState(()=>new BrowserTranslation());
 useEffect(()=>{if(new URLSearchParams(location.search).get('languages')==='all')void base.read().then(c=>setSnapshot(s=>parseDictation({...s,translationTargets:c.languages.map(l=>l.code)})));},[base]);
 const [port]=useState(()=>({read:()=>base.read(),save:base.save.bind(base),menu:async(_task:string,open:boolean)=>{setSnapshot(s=>({...s,translationMenu:open}));},select:async(task:string,code:string)=>{if(task!=='translation-fixture')throw Error('wrong task');setSnapshot(s=>({...s,targetLanguage:code}));}}));
 return <VoiceSurface presentation="recording" phase="recording" level={.4} translation={snapshot} translationPort={port} text="This raw transcript must not be rendered" start={()=>{}} finish={()=>setSnapshot(s=>({...s,translationSelectable:false,translationMenu:false}))} cancel={()=>{}}/>;
}
