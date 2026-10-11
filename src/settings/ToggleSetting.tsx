import {useId} from 'react';
import type {ProcessingSettings} from '../contracts/processing';
import {Button,InlineMessage,Switch} from '../ui';
import type {SettingStore} from './store';
import {useSetting} from './use-setting';
import s from './setting.module.css';
export function ToggleSetting({store,label,description,compact=false,headingLevel=3,titleId}:{store:SettingStore<ProcessingSettings>;label:string;description:string;compact?:boolean;headingLevel?:1|3;titleId?:string}){
 const state=useSetting(store),id=useId();
 const Heading=headingLevel===1?'h1':'h3';
 return <section className={`${s.setting} ${compact?s.compact:''} ${headingLevel===1?s.pageHeading:''}`} aria-label={label} aria-busy={state.busy}>
  <div className={s.row}><div><Heading id={titleId}><label htmlFor={id}>{label}</label></Heading><p id={id+'-description'} className={compact?s.hidden:undefined}>{description}</p></div><Switch id={id} aria-describedby={id+'-description'} label={label} checked={state.value?.enabled??false} disabled={!state.ready} onCheckedChange={enabled=>store.update({enabled})}/></div>
  {state.error&&<div><InlineMessage status="error">{state.error}</InlineMessage><Button variant="quiet" onClick={()=>void store.load()}>重新读取</Button></div>}
 </section>;
}
