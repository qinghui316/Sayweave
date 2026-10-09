import {useId} from 'react';
import type {ProcessingSettings} from '../contracts/processing';
import {Button,InlineMessage,Switch} from '../ui';
import type {SettingStore} from './store';
import {useSetting} from './use-setting';
import s from './setting.module.css';
export function ToggleSetting({store,label,description}:{store:SettingStore<ProcessingSettings>;label:string;description:string}){
 const state=useSetting(store),id=useId();
 return <section className={s.setting} aria-label={label} aria-busy={state.busy}>
  <div className={s.row}><div><h3><label htmlFor={id}>{label}</label></h3><p>{description}</p></div><Switch id={id} label={label} checked={state.value?.enabled??false} disabled={!state.ready} onCheckedChange={enabled=>store.update({enabled})}/></div>
  {state.error&&<div><InlineMessage status="error">{state.error}</InlineMessage><Button variant="quiet" onClick={()=>void store.load()}>重新读取</Button></div>}
 </section>;
}
