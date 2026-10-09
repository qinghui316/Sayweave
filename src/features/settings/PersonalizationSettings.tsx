import type {ProcessingSettings} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {ToggleSetting} from '../../settings/ToggleSetting';
export function PersonalizationSettings({store}:{store:SettingStore<ProcessingSettings>}){
 return <ToggleSetting store={store} label="个性化" description="按应用学习表达习惯并调整风格。使用所选文字模型，资料下次口述生效。"/>;
}
