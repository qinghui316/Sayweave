import type {LanguageSettingsPort} from '../../contracts/language';
import type {ProcessingSettings} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {PersonalizationSettings} from './PersonalizationSettings';
import s from './preferences.module.css';

export function Preferences({store,port,visible}:{store:SettingStore<ProcessingSettings>;port:LanguageSettingsPort;visible:boolean}) {
  return <section className={s.page} aria-labelledby="personalization-title">
    <PersonalizationSettings store={store} port={port} visible={visible}/>
  </section>;
}
