import {useEffect,useState} from 'react';
import type {ProcessingPort,ProcessingStatus,ProcessingSettings as Preference} from '../../contracts/processing';
import type {SettingStore} from '../../settings/store';
import {ToggleSetting} from '../../settings/ToggleSetting';
import s from './settings.module.css';
export function ProcessingSettings({port,store,revision}:{port:ProcessingPort;store:SettingStore<Preference>;revision:number}) {
 const [view,setView]=useState<ProcessingStatus|null>(null);
 useEffect(()=>{let alive=true;void port.status().then(value=>{if(alive)setView(value);}).catch(()=>{});return()=>{alive=false;};},[port,revision]);
 return <section className={s.processing}>
  <ToggleSetting store={store} label="整理口述" description="保留原意，让表达更清晰。内容会发送到所选文字服务。"/>
  <p className={s.processingModel}>{view?.model?'文字模型：'+view.model:'选择文字模型后可用。'}</p>{view?.error&&<p role="alert">请检查文字模型的配置和密钥。</p>}
 </section>;
}
