import type {LanguageSettingsPort} from '../../contracts/language';
import type {ProcessingSettings} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {PersonalizationSettings} from './PersonalizationSettings';
import s from './preferences.module.css';

export function Preferences({store,port,visible}:{store:SettingStore<ProcessingSettings>;port:LanguageSettingsPort;visible:boolean}) {
  return <section className={s.page} aria-labelledby="personalization-title">
    <header><h1 id="personalization-title">个性化</h1><p>查看按应用学到的表达习惯，让文字更像你。</p></header>
    <PersonalizationSettings store={store} port={port} visible={visible}/>
  </section>;
}
