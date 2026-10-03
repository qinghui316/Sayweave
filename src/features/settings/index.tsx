import { Dialog, Button, Icon } from '../../ui';
import { zh } from '../../locales/zh-CN';
import s from './settings.module.css';
export type SettingsTab = keyof typeof zh.settings.tabs;
export function SettingsShell({ open, tab, onTabChange, onOpenChange }: { open: boolean; tab: SettingsTab; onTabChange: (tab: SettingsTab) => void; onOpenChange: (open: boolean) => void }) {
  const t = zh.settings;
  const headings = { device: t.deviceHeading, models: t.modelsHeading, language: t.languageHeading, appearance: t.appearanceHeading, about: t.aboutHeading };
  return <Dialog open={open} onOpenChange={onOpenChange} title={t.title} description={t.description}>
    <div className={s.shell}><nav className={s.nav} aria-label={t.title}>{(Object.keys(t.tabs) as SettingsTab[]).map(key => <button key={key} type="button" className={`${s.item} ${tab === key ? s.selected : ''}`} onClick={() => onTabChange(key)} aria-current={tab === key ? 'page' : undefined}><Icon name={key === 'about' ? 'info' : key === 'models' ? 'square' : key === 'language' ? 'languages' : key === 'appearance' ? 'sliders-horizontal' : 'settings'} size={18}/>{t.tabs[key]}</button>)}</nav>
      <div className={s.content}><header><h2>{headings[tab]}</h2></header><div className={s.scroll} key={tab} tabIndex={0}>
        {tab === 'device' ? <><section><h3>{t.microphone}</h3><p>{t.microphoneHelp}</p><Button disabled>{t.notConfigured}</Button></section><section><h3>{zh.home.globalKeys}</h3>{[zh.home.dictation,zh.home.translation,zh.home.edit].map(mode => <div className={s.settingRow} key={mode}><div><h4>{mode}</h4><p>{t.notImplemented}</p></div><Button disabled>{t.notConfigured}</Button></div>)}</section></> :
        tab === 'about' ? <><section><h3>{t.version}</h3><p>{t.versionValue}</p></section>{[t.privacy,t.terms,t.license].map(label => <div className={s.aboutRow} key={label}><h3>{label}</h3><span>{t.pending}</span></div>)}</> :
        <section><p>{tab === 'models' ? t.modelsBody : tab === 'language' ? t.languageBody : t.appearanceBody}</p><p className={s.notice}>{t.notImplemented}</p></section>}
      </div></div>
    </div>
  </Dialog>;
}
