import { useCallback, useEffect, useRef, useState, useSyncExternalStore, lazy, Suspense } from 'react';
import { HashRouter, Link, Route, Routes,useNavigate,useLocation } from 'react-router-dom';
import {createSpeechPorts} from '../adapters/speech-tauri';
import {SpeechStore} from './speech-store';
import {speechError} from '../contracts/speech';
import {speechMessages} from '../locales/speech';
import type {Usage} from '../contracts/history';
import { createRuntime } from '../adapters/tauri';
import { createDictionary } from '../adapters/dictionary-tauri';
import { createModels } from '../adapters/models-tauri';
import { ModelPanel } from '../features/models';
import type { DemoAction, RuntimeErrorCode, WindowAction, Surface } from '../contracts/runtime';
import { RuntimeError } from '../contracts/runtime';
import { RuntimeStore } from './runtime-store';
import { ErrorBoundary } from './ErrorBoundary';
import { zh } from '../locales/zh-CN';
import { Button, Icon, IconButton, InlineMessage, NavItem, MotionScope } from '../ui';
import { Home } from '../features/home';
import { SettingsShell, type SettingsTab } from '../features/settings';
import { History } from '../features/history';
import { Dictionary } from '../features/dictionary';
import { Preferences } from '../features/preferences';
import { Onboarding } from '../features/onboarding';
import { VoiceSurface } from '../features/voice';
import brand from '../assets/brand.svg';
import s from './app.module.css';
const DevCatalogue = import.meta.env.DEV ? lazy(() => import('../dev/Catalogue')) : null;
const voiceDemo = import.meta.env.DEV && new URLSearchParams(window.location.search).get('voiceDemo')==='1';

function RuntimeApp() {
  const navigate=useNavigate();const location=useLocation();
  const [ports]=useState(createSpeechPorts);
  const [speechStore]=useState(()=>new SpeechStore(ports.speech));
  const speech=useSyncExternalStore(speechStore.subscribe,speechStore.getSnapshot);
  const [speechFailure,setSpeechFailure]=useState('');
  const [usage,setUsage]=useState<Usage|null>(null);
  const [store] = useState(() => new RuntimeStore(createRuntime()));
  const [dictionary] = useState(createDictionary);
  const [models] = useState(createModels);
  const modelGuard=useRef<((action:()=>void)=>void)|null>(null);
  const updateModelGuard=useCallback((guard:((action:()=>void)=>void)|null)=>{modelGuard.current=guard;},[]);
  const snapshot = useSyncExternalStore(store.subscribe,store.getSnapshot);
  const [error,setError]=useState<RuntimeErrorCode | null>(null);
  const [connection,setConnection]=useState(0);
  const [settingsOpen,setSettingsOpen]=useState(false);
  const [settingsTab,setSettingsTab]=useState<SettingsTab>('device');
  const report=useCallback((e: unknown) => setError(e instanceof RuntimeError ? e.code : 'unavailable'),[]);
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
  },[ports,store,speech.snapshot.lifecycle,location.pathname]);
  const startSpeech=()=>{setSpeechFailure('');void ports.speech.start().then(speechStore.accept).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));};
  const controlSpeech=(action:'finish'|'cancel')=>{const id=speech.snapshot.taskId;if(id)void ports.speech.control(id,action).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));};
  const demo=useCallback((action: DemoAction) => { setError(null); store.port.demo(action,store.getSnapshot().session.id).then(store.accept).catch(report); },[store,report]);
  const windowAction=useCallback((surface: Surface,action: WindowAction) => { store.port.windowAction(surface,action).then(store.accept).catch(report); },[store,report]);
  const openSettings=(tab: SettingsTab) => { setSettingsTab(tab); setSettingsOpen(true); };
  const phase=speech.snapshot.phase;
  if (store.port.surface==='overlay') return <MotionScope enabled={snapshot.windows.overlay}><VoiceSurface retry={voiceDemo?false:speech.snapshot.retry} error={voiceDemo?undefined:speechFailure||(speech.snapshot.error?speechMessages[speech.snapshot.error]:undefined)} level={voiceDemo?undefined:speech.level} motionEnabled={snapshot.windows.overlay} phase={voiceDemo?snapshot.session.phase:phase} start={voiceDemo?()=>demo('start'):startSpeech} cancel={voiceDemo?()=>demo('cancel'):()=>controlSpeech('cancel')} finish={voiceDemo?()=>demo('finish'):()=>controlSpeech('finish')}/></MotionScope>;
  return <MotionScope enabled={snapshot.windows.main}><div className={s.window}>
    <aside className={s.sidebar}><div className={s.brand} data-tauri-drag-region><img src={brand} alt="" width={24} height={24}/><span>{zh.brand}</span></div>
      <nav className={s.navigation} aria-label={zh.navigation.home}><NavItem to="/" icon="house">{zh.navigation.home}</NavItem><NavItem to="/history" icon="clock">{zh.navigation.history}</NavItem><NavItem to="/dictionary" icon="book-open">{zh.navigation.dictionary}</NavItem><NavItem to="/preferences" icon="sliders-horizontal">{zh.navigation.preferences}</NavItem></nav>
      <footer className={s.sidebarFooter}>{import.meta.env.DEV && <Link to="/dev/components" className={s.devEntry}>{zh.dev.component}</Link>}<div className={s.utility}><button onClick={()=>openSettings('device')}><Icon name="settings" size={18}/>{zh.navigation.settings}</button><IconButton label={zh.navigation.about} onClick={()=>openSettings('about')}><Icon name="info" size={18}/></IconButton></div></footer>
    </aside>
    <div className={s.body}><header className={s.titlebar} data-tauri-drag-region><IconButton label={zh.window.minimize} onClick={()=>windowAction('main','minimize')}><Icon name="minus" size={16}/></IconButton><IconButton label={zh.window.maximize} onClick={()=>windowAction('main','toggleMaximize')}><Icon name="square" size={16}/></IconButton><IconButton label={zh.window.close} onClick={()=>windowAction('main','hide')}><Icon name="x" size={16}/></IconButton></header>
      <main className={s.content}>{speechFailure&&!speech.snapshot.error&&<InlineMessage status="error">{speechFailure}</InlineMessage>}{error && <div className={s.error}><InlineMessage status="error">{zh.error[error]}</InlineMessage><Button onClick={()=>{setError(null);setConnection(x=>x+1);}}>{zh.error.retry}</Button></div>}
        <MotionScope enabled={!settingsOpen}><Routes><Route path="/" element={<Home motionEnabled={snapshot.windows.main && !settingsOpen} onShortcuts={()=>openSettings('device')} onModels={()=>openSettings('models')} retryCurrent={()=>{setSpeechFailure('');void ports.speech.retryCurrent().then(speechStore.accept).catch(e=>setSpeechFailure(speechMessages[speechError(e).code]));}} speech={speech.snapshot} level={speech.level} usage={usage} simulation={ports.speech.mode==='simulation'} copy={text=>ports.speech.copy(text)} phase={phase} start={startSpeech} cancel={()=>controlSpeech('cancel')} finish={()=>controlSpeech('finish')}/>}/><Route path="/history" element={<History port={ports.history} speech={ports.speech} models={models} snapshot={speech.snapshot}/>}/><Route path="/dictionary" element={<Dictionary port={dictionary}/>}/><Route path="/preferences" element={<Preferences/>}/><Route path="/onboarding" element={<Onboarding/>}/>{DevCatalogue && <Route path="/dev/components" element={<Suspense><DevCatalogue snapshot={snapshot} demo={demo} windowAction={windowAction} resync={()=>store.resync().catch(report)}/></Suspense>}/>}<Route path="*" element={<History port={ports.history} speech={ports.speech} models={models} snapshot={speech.snapshot}/>}/></Routes></MotionScope>
      </main>
    </div>
    <SettingsShell speechPort={ports.speech} open={settingsOpen} tab={settingsTab}
      onTabChange={tab=>{const action=()=>setSettingsTab(tab);if(modelGuard.current)modelGuard.current(action);else action();}}
      onOpenChange={open=>{const action=()=>setSettingsOpen(open);if(!open&&modelGuard.current)modelGuard.current(action);else action();}}
      modelContent={settingsOpen&&settingsTab==='models'?<ModelPanel onSpeech={()=>{setSettingsOpen(false);navigate('/');}} port={models} onGuardChange={updateModelGuard}/>:undefined}/>
  </div></MotionScope>;
}
export function App() { return <ErrorBoundary><HashRouter><RuntimeApp/></HashRouter></ErrorBoundary>; }

