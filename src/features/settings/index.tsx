import {TranslationSettings} from './TranslationSettings';
import type {TranslationPort} from '../../contracts/translation';
import {AppearanceSettings} from './AppearanceSettings';
import { Dialog, Button, Icon, RubberIndicator, useRubberSelection } from '../../ui';
import { zh } from '../../locales/zh-CN';
import s from './settings.module.css';
import type {ReactNode} from 'react';
import type {SpeechPort} from '../../contracts/speech';
import {DeviceSettings} from './DeviceSettings';
import {ShortcutSettings} from './ShortcutSettings';
import type {ShortcutConfigPort} from '../../contracts/dictation';
import type {TextServicePort} from '../../contracts/text-service';
import {TextServiceSettings} from './TextServiceSettings';
import type {SettingsStores} from '../../settings/controllers';
import type {UpdatePort} from '../../contracts/update';
import {UpdatePanel} from '../../ui/UpdatePanel';
export type SettingsTab = keyof typeof zh.settings.tabs;
export {ProcessingSettings} from './ProcessingSettings';
export function SettingsShell({ open, tab, onTabChange, onOpenChange, modelContent,speechPort,shortcutPort,textServicePort,translationPort,settings,updatePort,onInstallUpdate,updatePreparing=false,updateError=null }: { open: boolean; tab: SettingsTab; onTabChange: (tab: SettingsTab) => void; onOpenChange: (open: boolean) => void; modelContent?:ReactNode;speechPort?:SpeechPort;shortcutPort?:ShortcutConfigPort;textServicePort?:TextServicePort;translationPort?:TranslationPort;settings:SettingsStores;updatePort:UpdatePort;onInstallUpdate:()=>void;updatePreparing?:boolean;updateError?:string|null }) {
  const t = zh.settings;
  const {ref:navRef}=useRubberSelection<HTMLElement>(tab);
  const headings = { device: t.deviceHeading, models: t.modelsHeading, language: t.languageHeading, appearance: t.appearanceHeading, about: t.aboutHeading };
  return <Dialog open={open} onOpenChange={onOpenChange} title={t.title} description={t.description}>
    <div className={s.shell}><nav ref={navRef} data-rubber="" className={s.nav} aria-label={t.title}><RubberIndicator/>{(Object.keys(t.tabs) as SettingsTab[]).map(key => <button key={key} type="button" className={`${s.item} ${tab === key ? s.selected : ''}`} onClick={() => onTabChange(key)} aria-current={tab === key ? 'page' : undefined}><Icon signal={tab===key} name={key === 'about' ? 'info' : key === 'models' ? 'model' : key === 'appearance' ? 'palette' : 'settings'} size={18}/>{t.tabs[key]}</button>)}</nav>
      <div className={s.content}><header><h2>{headings[tab]}</h2></header>{tab === 'models' && modelContent ? <div className={s.panelSlot}>{modelContent}</div> : <div className={s.scroll} key={tab} tabIndex={0}>
        {tab === 'device' ? <>{speechPort?<DeviceSettings port={speechPort} settings={settings.recording}/>:<section><h3>{t.microphone}</h3><p>{t.microphoneHelp}</p><Button disabled>{t.notConfigured}</Button></section>}{shortcutPort&&<ShortcutSettings port={shortcutPort} settings={settings.shortcuts}/>} {textServicePort&&<TextServiceSettings port={textServicePort}/>}</> :
        tab === 'about' ? <><UpdatePanel port={updatePort} onInstall={onInstallUpdate} preparing={updatePreparing} error={updateError}/><div className={s.aboutRow}><h3>{t.license}</h3><span>Apache-2.0</span></div></> :
        tab === 'language' && translationPort ? <TranslationSettings port={translationPort} store={settings.translation}/> :
        tab === 'appearance' ? <AppearanceSettings store={settings.appearance}/> : tab === 'models' && modelContent ? modelContent :
        <section><p>{tab === 'models' ? t.modelsBody : t.appearanceBody}</p><p className={s.notice}>{t.notImplemented}</p></section>}
      </div>}</div>
    </div>
  </Dialog>;
}
