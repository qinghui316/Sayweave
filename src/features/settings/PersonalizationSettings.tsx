import type {ProcessingSettings} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {ToggleSetting} from '../../settings/ToggleSetting';
import type {LanguageSettingsPort} from '../../contracts/language';
import {PersonalizationDashboard} from './PersonalizationDashboard';
export function PersonalizationSettings({store,port,visible}:{store:SettingStore<ProcessingSettings>;port:LanguageSettingsPort;visible:boolean}){
 return <><ToggleSetting store={store} label="个性化" description="按应用学习表达习惯，资料下次口述生效。"/><PersonalizationDashboard port={port} visible={visible}/></>;
}
