import { useCallback, useEffect, useState, useSyncExternalStore, lazy, Suspense } from 'react';
import { HashRouter, Link, Route, Routes } from 'react-router-dom';
import { createRuntime } from '../adapters/tauri';
import { createDictionary } from '../adapters/dictionary-tauri';
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

function RuntimeApp() {
  const [store] = useState(() => new RuntimeStore(createRuntime()));
  const [dictionary] = useState(createDictionary);
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
  const demo=useCallback((action: DemoAction) => { setError(null); store.port.demo(action,store.getSnapshot().session.id).then(store.accept).catch(report); },[store,report]);
  const windowAction=useCallback((surface: Surface,action: WindowAction) => { store.port.windowAction(surface,action).then(store.accept).catch(report); },[store,report]);
  const openSettings=(tab: SettingsTab) => { setSettingsTab(tab); setSettingsOpen(true); };
  const phase=snapshot.session.phase;
  if (store.port.surface==='overlay') return <MotionScope enabled={snapshot.windows.overlay}><VoiceSurface motionEnabled={snapshot.windows.overlay} phase={phase} start={()=>demo('start')} cancel={()=>demo('cancel')} finish={()=>demo('finish')}/></MotionScope>;
  return <MotionScope enabled={snapshot.windows.main}><div className={s.window}>
    <aside className={s.sidebar}><div className={s.brand} data-tauri-drag-region><img src={brand} alt="" width={24} height={24}/><span>{zh.brand}</span></div>
      <nav className={s.navigation} aria-label={zh.navigation.home}><NavItem to="/" icon="house">{zh.navigation.home}</NavItem><NavItem to="/history" icon="clock">{zh.navigation.history}</NavItem><NavItem to="/dictionary" icon="book-open">{zh.navigation.dictionary}</NavItem><NavItem to="/preferences" icon="sliders-horizontal">{zh.navigation.preferences}</NavItem></nav>
      <footer className={s.sidebarFooter}>{import.meta.env.DEV && <Link to="/dev/components" className={s.devEntry}>{zh.dev.component}</Link>}<div className={s.utility}><button onClick={()=>openSettings('device')}><Icon name="settings" size={18}/>{zh.navigation.settings}</button><IconButton label={zh.navigation.about} onClick={()=>openSettings('about')}><Icon name="info" size={18}/></IconButton></div></footer>
    </aside>
    <div className={s.body}><header className={s.titlebar} data-tauri-drag-region><IconButton label={zh.window.minimize} onClick={()=>windowAction('main','minimize')}><Icon name="minus" size={16}/></IconButton><IconButton label={zh.window.maximize} onClick={()=>windowAction('main','toggleMaximize')}><Icon name="square" size={16}/></IconButton><IconButton label={zh.window.close} onClick={()=>windowAction('main','hide')}><Icon name="x" size={16}/></IconButton></header>
      <main className={s.content}>{error && <div className={s.error}><InlineMessage status="error">{zh.error[error]}</InlineMessage><Button onClick={()=>{setError(null);setConnection(x=>x+1);}}>{zh.error.retry}</Button></div>}
        <MotionScope enabled={!settingsOpen}><Routes><Route path="/" element={<Home motionEnabled={snapshot.windows.main && !settingsOpen} onShortcuts={()=>openSettings('device')} onModels={()=>openSettings('models')} phase={phase} start={()=>demo('start')} cancel={()=>demo('cancel')} finish={()=>demo('finish')}/>}/><Route path="/history" element={<History/>}/><Route path="/dictionary" element={<Dictionary port={dictionary}/>}/><Route path="/preferences" element={<Preferences/>}/><Route path="/onboarding" element={<Onboarding/>}/>{DevCatalogue && <Route path="/dev/components" element={<Suspense><DevCatalogue snapshot={snapshot} demo={demo} windowAction={windowAction} resync={()=>store.resync().catch(report)}/></Suspense>}/>}<Route path="*" element={<History/>}/></Routes></MotionScope>
      </main>
    </div>
    <SettingsShell open={settingsOpen} tab={settingsTab} onTabChange={setSettingsTab} onOpenChange={setSettingsOpen}/>
  </div></MotionScope>;
}
export function App() { return <ErrorBoundary><HashRouter><RuntimeApp/></HashRouter></ErrorBoundary>; }

