import type {ProcessingSettings} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {Button} from '../../ui';
import {ToggleSetting} from '../../settings/ToggleSetting';
import type {LanguageSettingsPort} from '../../contracts/language';
import {PersonalizationDashboard} from './PersonalizationDashboard';
export function PersonalizationSettings({store,port,visible,onModels}:{store:SettingStore<ProcessingSettings>;port:LanguageSettingsPort;visible:boolean;onModels?:()=>void}){
 return <><ToggleSetting store={store} label="个性化" description="按应用学习表达习惯，资料下次口述生效。" headingLevel={1} titleId="personalization-title"/><p>学习需配置默认文字模型，分析使用该模型服务。持续口述并纠正文字后，资料会逐步积累。{onModels&&<Button variant="quiet" onClick={onModels}>文字模型设置</Button>}</p><PersonalizationDashboard port={port} visible={visible}/></>;
}
