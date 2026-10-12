import {dictationFeedback} from './dictation-feedback';
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, lazy, Suspense } from 'react';
import { HashRouter, Link, Route, Routes,useNavigate,useLocation } from 'react-router-dom';
import {createSpeechPorts} from '../adapters/speech-tauri';
import {SpeechStore} from './speech-store';
import {createDictationPorts} from '../adapters/dictation-tauri';
import {createProcessing} from '../adapters/processing-tauri';
import {createLanguageSettings} from '../adapters/language-tauri';
import {createSettingsStores} from '../settings/controllers';
import {dictationPhase,initialDictation,inputMessage,shortcutLabel,type ShortcutStatus} from '../contracts/dictation';
import {speechError} from '../contracts/speech';
import {readReadyModel} from './ready-model';
import {speechMessages} from '../locales/speech';
import type {Usage} from '../contracts/history';
import { createRuntime } from '../adapters/tauri';
import { createDictionary } from '../adapters/dictionary-tauri';
import { createModels, openModelHelp } from '../adapters/models-tauri';
import { ModelPanel } from '../features/models';
import type { DemoAction, RuntimeErrorCode, WindowAction, Surface } from '../contracts/runtime';
import { RuntimeError } from '../contracts/runtime';
import { RuntimeStore } from './runtime-store';
import { ErrorBoundary } from './ErrorBoundary';
import { zh } from '../locales/zh-CN';
import { Button, Icon, IconButton, InlineMessage, NavItem, MotionScope, EmptyState, RubberIndicator, useRubberSelection } from '../ui';
import { Home } from '../features/home';
import { SettingsShell,ProcessingSettings, type SettingsTab } from '../features/settings';
import { History, HistoryFeedStore } from '../features/history';
import { Dictionary } from '../features/dictionary';
import { Onboarding } from '../features/onboarding';
import { VoiceSurface } from '../features/voice';
import {createUpdatePort} from '../adapters/update-tauri';
import {UpdateDock} from '../ui/UpdateDock';
import {Brand} from '../ui/Brand';
import {Preferences} from '../features/preferences';
import s from './app.module.css';
const DevCatalogue = import.meta.env.DEV ? lazy(() => import('../dev/Catalogue')) : null;
const voiceDemo = import.meta.env.DEV && new URLSearchParams(window.location.search).get('voiceDemo')==='1';

function RuntimeApp() {
  const navigate=useNavigate();const location=useLocation();
  const [ports]=useState(createSpeechPorts);
  const [dictationPorts]=useState(createDictationPorts);
  const [dictation,setDictation]=useState(initialDictation);
  const [shortcut,setShortcut]=useState<ShortcutStatus|null>(null);
  useEffect(()=>{let alive=true;const releases:(()=>void)[]=[];const accept=(v:ReturnType<typeof initialDictation>)=>{if(alive)setDictation(old=>v.sequence>=old.sequence?v:old);};
    if(dictationPorts.dictation){void dictationPorts.dictation.subscribe(accept).then(r=>{if(alive)releases.push(r);else r();}).catch(()=>{});void dictationPorts.dictation.snapshot().then(accept).catch(()=>{});}
    let shortcutSequence=-1;const keys=(v:ShortcutStatus)=>{if(alive&&v.sequence>shortcutSequence){shortcutSequence=v.sequence;setShortcut(v);}};void dictationPorts.shortcuts.subscribe(keys).then(r=>{if(alive)releases.push(r);else r();}).catch(()=>{});void dictationPorts.shortcuts.status().then(keys).catch(()=>{});
    return()=>{alive=false;for(const r of releases)r();};},[dictationPorts]);
  const [reading]=useState(()=>new HistoryFeedStore(ports.history));
  const [speechStore]=useState(()=>new SpeechStore(ports.speech));
  const speech=useSyncExternalStore(speechStore.subscribe,speechStore.getSnapshot);
  const trial=useSyncExternalStore(speechStore.subscribeTrial,speechStore.getTrialSnapshot);
  const {ref:navRef}=useRubberSelection<HTMLElement>(location.pathname);
  const [trialFailure,setTrialFailure]=useState('');
  const [trialPending,setTrialPending]=useState(false);
  const trialRequest=useRef(0);
  const [speechFailure,setSpeechFailure]=useState('');
  const [overlayFailure,setOverlayFailure]=useState<{taskId:string;message:string}|null>(null);
  const [usage,setUsage]=useState<Usage|null>(null);
  const [store] = useState(() => new RuntimeStore(createRuntime()));
  const [dictionary] = useState(createDictionary);
  const [models] = useState(createModels);
  const [updates] = useState(createUpdatePort);
  const [processing]=useState(()=>createProcessing(models));
  const [language]=useState(createLanguageSettings);
  const [settings]=useState(()=>createSettingsStores(processing,language,ports.speech,dictationPorts.shortcuts));
  const [dataRevision,setDataRevision]=useState(0);
  const dataChanged=useCallback(()=>setDataRevision(value=>value+1),[]);
  useEffect(()=>{if(!processing.onArchived)return;let stopped=false;let release:(()=>void)|undefined;void processing.onArchived(dataChanged).then(value=>{if(stopped)value();else release=value;});return()=>{stopped=true;release?.();};},[processing,dataChanged]);
  const [ready,setReady]=useState({model:'',device:''});
  const modelGuard=useRef<((action:()=>void)=>void)|null>(null);
  const dictionaryGuard=useRef<((action:()=>void)=>void)|null>(null);
  const updateDictionaryGuard=useCallback((guard:((action:()=>void)=>void)|null)=>{dictionaryGuard.current=guard;},[]);
  const [updatePreparing,setUpdatePreparing]=useState(false),[updateError,setUpdateError]=useState<string|null>(null);
  const updateFlight=useRef(false);
  const installUpdate=()=>{
    if(updateFlight.current)return;
    const candidate=updates.snapshot();if(candidate.state!=='readyToInstall'||!candidate.taskId)return;
    const install=()=>{
      if(updateFlight.current)return;updateFlight.current=true;setUpdatePreparing(true);setUpdateError(null);
      const stores=Object.values(settings);stores.forEach(value=>value.pause());
      void (async()=>{
        let timer:ReturnType<typeof setTimeout>|undefined;
        try{await Promise.race([Promise.all(stores.map(value=>value.flush())),new Promise<never>((_,reject)=>{timer=setTimeout(()=>reject(new Error('设置保存超时，请确认后重试。')),5000);})]);await updates.install(candidate.taskId!,candidate.revision);}
        catch(error){setUpdateError(error instanceof Error?error.message:'更新准备未完成，请重试。');}
        finally{clearTimeout(timer);if(!updates.snapshot().recovery)stores.forEach(value=>value.resume());updateFlight.current=false;setUpdatePreparing(false);}
      })();
    };
    const afterModel=()=>dictionaryGuard.current?dictionaryGuard.current(install):install();
    if(modelGuard.current)modelGuard.current(afterModel);else afterModel();
  };
  const updateModelGuard=useCallback((guard:((action:()=>void)=>void)|null)=>{modelGuard.current=guard;},[]);
  const snapshot = useSyncExternalStore(store.subscribe,store.getSnapshot);
  const [error,setError]=useState<RuntimeErrorCode | null>(null);
  const [connection,setConnection]=useState(0);
  const [settingsOpen,setSettingsOpen]=useState(false);
  const [settingsTab,setSettingsTab]=useState<SettingsTab>('device');
  useEffect(()=>{
    if(store.port.surface!=='main')return;
    const refresh=()=>{if(!document.hidden)for(const value of Object.values(settings))void value.load();};
    refresh();window.addEventListener('focus',refresh);document.addEventListener('visibilitychange',refresh);
    return()=>{window.removeEventListener('focus',refresh);document.removeEventListener('visibilitychange',refresh);};
  },[settings,store]);
  const settingsVisible=settingsOpen,activeSettingsTab=settingsTab;
  const report=useCallback((e: unknown) => setError(e instanceof RuntimeError ? e.code : 'unavailable'),[]);
  useEffect(()=>{if(!dictationPorts.dictation||store.port.surface!=='main')return;let disposed=false;let release:(()=>void)|undefined;void dictationPorts.dictation.onSettings(()=>{setSettingsTab('device');setSettingsOpen(true);}).then(r=>{if(disposed)r();else release=r;}).catch(report);return()=>{disposed=true;release?.();};},[dictationPorts,store,report]);
  useEffect(() => {
    let stopped=false; let release: (()=>void) | undefined;
    store.connect().then(unlisten => { if (stopped) unlisten(); else release=unlisten; }).catch(report);
    const resync=()=>{ if (!document.hidden) store.resync().catch(report); };
    window.addEventListener('focus',resync); document.addEventListener('visibilitychange',resync);
    return () => { stopped=true; release?.(); window.removeEventListener('focus',resync); document.removeEventListener('visibilitychange',resync); };
  },[store,report,connection]);
  useEffect(()=>{
    let stopped=false;let release:(()=>void)|undefined;
    speechStore.connect().then(r=>{if(stopped)r();else release=r;}).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));
    const resync=()=>{if(!document.hidden)void speechStore.resync().catch(()=>{});};window.addEventListener('focus',resync);
    return()=>{stopped=true;release?.();window.removeEventListener('focus',resync);};
  },[speechStore]);
  useEffect(()=>{if(store.port.surface==='overlay')return;let stopped=false;let timer:ReturnType<typeof setTimeout>|undefined;
    async function readUsage(){try{const status=await ports.history.status();if(status.state==='preparing'){timer=setTimeout(()=>void readUsage(),100);return;}const value=await ports.history.usage();if(!stopped)setUsage(value);}catch{if(!stopped)setUsage(null);}}
    void readUsage();return()=>{stopped=true;clearTimeout(timer);};
  },[ports,store,speech.snapshot.saved,speech.snapshot.attemptId,dataRevision,location.pathname]);
  useEffect(()=>{
    if(store.port.surface==='overlay'||settingsOpen)return;
    let stopped=false;let sequence=0;
    async function read(){const token=++sequence;try{
      const value=await readReadyModel(models,ports.speech,()=>!stopped&&token===sequence);
      if(value&&!stopped&&token===sequence)setReady(value);
    }catch{if(!stopped&&token===sequence)setReady({model:'',device:''});}}
    void read();const focus=()=>{if(!document.hidden)void read();};window.addEventListener('focus',focus);
    return()=>{stopped=true;window.removeEventListener('focus',focus);};
  },[ports,store,models,dataRevision,settingsOpen]);
  const startSpeech=()=>{setSpeechFailure('');void ports.speech.start().then(speechStore.accept).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));};
  const controlSpeech=(action:'finish'|'cancel')=>{const id=speech.snapshot.taskId;if(id)void ports.speech.control(id,action).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));};
  const occupied=['connecting','recording','processing'].includes(speech.snapshot.phase)||['preparing','recognizing','delivering'].includes(dictation.stage)||dictation.stage==='result'&&['recognizing','delivering'].includes(dictation.presentation);
  const trialCurrent=trial.snapshot.taskId!==null&&speech.snapshot.taskId===trial.snapshot.taskId&&speech.snapshot.source==='trial'&&(!dictationPorts.dictation||dictation.taskId===trial.snapshot.taskId);
  const trialPhase=!trialCurrent&&['connecting','recording','processing'].includes(trial.snapshot.phase)?'idle':trial.snapshot.phase;
  const startTrial=()=>{
    if(occupied||trialPending)return;
    const request=++trialRequest.current;setTrialFailure('');setTrialPending(true);
    void ports.speech.start().then(speechStore.accept).catch(e=>{if(request===trialRequest.current)setTrialFailure(speechMessages[speechError(e).code]);}).finally(()=>{if(request===trialRequest.current)setTrialPending(false);});
  };
  const trialAction=(action:'finish'|'cancel'|'retry')=>{
    const id=trial.snapshot.taskId,current=speechStore.getSnapshot().snapshot;
    if(!trialCurrent||!id||current.taskId!==id||current.source!=='trial'||action==='retry'&&(occupied||trialPending||!current.canRetry))return;
    const request=++trialRequest.current;setTrialFailure('');
    if(action==='retry')setTrialPending(true);
    const operation=action==='retry'?ports.speech.retryCurrent(id).then(speechStore.accept):ports.speech.control(id,action);
    void operation.catch(e=>{if(action!=='retry'&&speechError(e).code==='staleSession')return;if(request===trialRequest.current&&speechStore.getSnapshot().snapshot.source==='trial'&&speechStore.getTrialSnapshot().snapshot.taskId===id)setTrialFailure(speechMessages[speechError(e).code]);}).finally(()=>{if(request===trialRequest.current)setTrialPending(false);});
  };
  const demo=useCallback((action: DemoAction) => { setError(null); store.port.demo(action,store.getSnapshot().session.id).then(store.accept).catch(report); },[store,report]);
  const windowAction=useCallback((surface: Surface,action: WindowAction) => { store.port.windowAction(surface,action).then(store.accept).catch(report); },[store,report]);
  const openSettings=(tab: SettingsTab) => {setSettingsTab(tab);setSettingsOpen(true);};
  const guardSettings=(action:()=>void)=>{if(modelGuard.current)modelGuard.current(action);else action();};
  const phase=speech.snapshot.phase;
  const overlayPhase=dictationPhase(dictation,phase,speech.snapshot);
  const overlayStart=()=>{if(dictationPorts.dictation)void dictationPorts.dictation.start().then(speechStore.accept).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));else startSpeech();};
  const overlayControl=(action:'finish'|'cancel')=>{if(dictationPorts.dictation){if(dictation.taskId)void dictationPorts.dictation.control(dictation.taskId,action).catch(e=>{if(e&&typeof e==='object'&&'code'in e&&e.code==='staleSession')return;setOverlayFailure({taskId:dictation.taskId!,message:inputMessage(e&&typeof e==='object'&&'code'in e?String(e.code):null)});});}else controlSpeech(action);};
  if (store.port.surface==='overlay') return <MotionScope enabled={snapshot.windows.overlay}><VoiceSurface configure={dictation.admission==='rejected'&&dictation.taskId&&dictation.presentationId?()=>{void dictationPorts.dictation?.settings(dictation.taskId!,dictation.presentationId!).catch(report);}:undefined} key={dictation.presentationId??'idle'} presentation={voiceDemo?undefined:dictation.source==='external'?dictation.presentation:undefined} dismiss={dictationPorts.dictation&&dictation.taskId&&dictation.presentationId?()=>{void dictationPorts.dictation!.dismiss(dictation.taskId!,dictation.presentationId!).catch(e=>{if(e&&typeof e==='object'&&'code'in e&&e.code==='staleSession')return;report(e);});}:undefined} mode={voiceDemo?'demo':'speech'} cancelLabel={!voiceDemo&&dictation.source==='external'&&speech.snapshot.taskId===dictation.taskId&&speech.snapshot.phase==='completed'?'停止填入':undefined} retry={voiceDemo?false:speech.snapshot.retry} error={voiceDemo?undefined:dictationFeedback(dictation,speech.snapshot)||(overlayFailure?.taskId===dictation.taskId&&dictation.presentation==='error'?overlayFailure.message:undefined)} meterSource={voiceDemo?undefined:speechStore.liveMeter} motionEnabled={snapshot.windows.overlay} phase={voiceDemo?snapshot.session.phase:overlayPhase} start={voiceDemo?()=>demo('start'):overlayStart} cancel={voiceDemo?()=>demo('cancel'):()=>overlayControl('cancel')} finish={voiceDemo?()=>demo('finish'):()=>overlayControl('finish')} text={dictation.source==='external'?(speech.snapshot.taskId===dictation.taskId?speech.snapshot.text:dictation.text):undefined} delivery={dictation.delivery} copy={dictationPorts.dictation?()=>dictation.taskId?dictation.presentationId?dictationPorts.dictation!.copy(dictation.taskId,dictation.presentationId):Promise.reject({code:'staleSession'}):Promise.reject({code:'staleSession'}):undefined}/></MotionScope>;
  const trialQuiet=trial.snapshot.source==='trial'&&dictation.source==='trial'&&dictation.taskId===trial.snapshot.taskId&&dictation.stage==='result'&&dictation.presentation==='idle'&&!trial.snapshot.saveError;
  const home=<Home quietTerminal={trialQuiet} shortcut={shortcut?.active?shortcutLabel(shortcut.active.binding):undefined} readyModel={ready.model} readyDevice={ready.device} motionEnabled={snapshot.windows.main && !settingsVisible} onShortcuts={()=>openSettings('device')} onModels={()=>openSettings('models')} retryCurrent={trialCurrent&&!occupied?()=>trialAction('retry'):undefined} startDisabled={occupied||trialPending} failure={trialFailure} speech={trial.snapshot} meterSource={speechStore.trialMeter} usage={usage} simulation={ports.speech.mode==='simulation'} copy={text=>ports.speech.copy(text)} phase={trialPhase} start={startTrial} cancel={()=>trialAction('cancel')} finish={()=>trialAction('finish')}/>;
  return <MotionScope enabled={snapshot.windows.main}><div className={s.window}>
    <aside className={s.sidebar}><div className={s.brand} data-tauri-drag-region><Brand/></div>
      <nav ref={navRef} data-rubber="" className={s.navigation} aria-label={zh.navigation.home}><RubberIndicator/><NavItem to="/" icon="house">{zh.navigation.home}</NavItem><NavItem to="/history" icon="clock">{zh.navigation.history}</NavItem><NavItem to="/dictionary" icon="book-open">{zh.navigation.dictionary}</NavItem><NavItem to="/preferences" icon="personalization">{zh.navigation.preferences}</NavItem></nav>
      <footer className={s.sidebarFooter}>{import.meta.env.DEV && <Link to="/dev/components" className={s.devEntry}>{zh.dev.component}</Link>}<div className={s.utility}><button onClick={()=>openSettings('device')}><Icon name="settings" size={18}/>{zh.navigation.settings}</button><UpdateDock port={updates} onAbout={()=>openSettings('about')} onInstall={installUpdate} preparing={updatePreparing} error={updateError} dismissKey={location.pathname+':'+settingsVisible}/></div></footer>
    </aside>
    <div className={s.body}><header className={s.titlebar} data-tauri-drag-region><IconButton label={zh.window.minimize} onClick={()=>windowAction('main','minimize')}><Icon name="minus" size={16}/></IconButton><IconButton label={zh.window.maximize} onClick={()=>windowAction('main','toggleMaximize')}><Icon name="square" size={16}/></IconButton><IconButton label={zh.window.close} onClick={()=>windowAction('main','hide')}><Icon name="x" size={16}/></IconButton></header>
      <main className={`${s.content} ${location.pathname==='/history'?s.historyContent:''}`}>{speechFailure&&!speech.snapshot.error&&<InlineMessage status="error">{speechFailure}</InlineMessage>}{error && <div className={s.error}><InlineMessage status="error">{zh.error[error]}</InlineMessage><Button onClick={()=>{setError(null);setConnection(x=>x+1);}}>{zh.error.retry}</Button></div>}
        <MotionScope enabled={!settingsVisible}><Routes><Route path="/" element={home}/><Route path="/history" element={<History contentRevision={dataRevision} deliveryRevision={dictation.delivery?[dictation.taskId,dictation.delivery.state,dictation.delivery.error].join(':'):''} reading={reading} visible={snapshot.windows.main&&!settingsOpen} onChanged={dataChanged} recordingSettings={settings.recording} port={ports.history} speech={ports.speech} models={models} snapshot={speech.snapshot}/>}/><Route path="/dictionary" element={<Dictionary port={dictionary} automaticTerms={settings.automaticTerms} onGuardChange={updateDictionaryGuard}/>}/><Route path="/preferences" element={<Preferences store={settings.personalization} port={language} visible={snapshot.windows.main&&!settingsVisible}/>}/><Route path="/onboarding" element={<Onboarding/>}/>{DevCatalogue && <Route path="/dev/components" element={<Suspense><DevCatalogue snapshot={snapshot} demo={demo} windowAction={windowAction} resync={()=>store.resync().catch(report)}/></Suspense>}/>}<Route path="/translation" element={<EmptyState title="翻译规划中" description="语音翻译功能正在规划。"><Button onClick={()=>navigate('/')}>返回首页</Button></EmptyState>}/><Route path="/edit" element={<EmptyState title="编辑规划中" description="语音编辑功能正在规划。"><Button onClick={()=>navigate('/')}>返回首页</Button></EmptyState>}/><Route path="*" element={<EmptyState title="页面未找到" description="这个地址暂时没有对应页面。"><Button onClick={()=>navigate('/')}>返回首页</Button></EmptyState>}/></Routes></MotionScope>
      </main>
    </div>
    <SettingsShell shortcutPort={dictationPorts.shortcuts} textServicePort={dictationPorts.textService} settings={settings} speechPort={ports.speech} open={settingsVisible} tab={activeSettingsTab}
      onTabChange={tab=>guardSettings(()=>setSettingsTab(tab))}
      onOpenChange={open=>{if(!open)guardSettings(()=>setSettingsOpen(false));else setSettingsOpen(true);}}
      updatePort={updates} onInstallUpdate={installUpdate} updatePreparing={updatePreparing} updateError={updateError} modelContent={settingsVisible&&activeSettingsTab==='models'?<ModelPanel extraContent={<ProcessingSettings port={processing} store={settings.polish} revision={dataRevision}/>} onChanged={dataChanged} onHelp={openModelHelp} onSpeech={()=>{setSettingsOpen(false);navigate('/');}} port={models} onGuardChange={updateModelGuard}/>:undefined}/>
  </div></MotionScope>;
}
export function App() { return <ErrorBoundary><HashRouter><RuntimeApp/></HashRouter></ErrorBoundary>; }

