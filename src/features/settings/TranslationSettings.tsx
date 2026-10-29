import {useEffect,useState} from 'react';
import {Button,Icon,IconButton,InlineMessage,Select,SettingsRow,SettingsSection} from '../../ui';
import type {TranslationLanguage,TranslationPort,TranslationSettings as Configuration} from '../../contracts/translation';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import s from './settings.module.css';

export function TranslationSettings({port,store}:{port:TranslationPort;store:SettingStore<Configuration>}) {
 const state=useSetting(store),value=state.value;
 const [languages,setLanguages]=useState<TranslationLanguage[]>([]),[error,setError]=useState(''),[reload,setReload]=useState(0);
 useEffect(()=>{let alive=true;void port.read().then(c=>{if(alive){setLanguages(c.languages);store.accept(c.settings);setError('');}}).catch(()=>{if(alive)setError('无法读取语言，请重试。');});return()=>{alive=false;};},[port,store,reload]);
 const label=(code:string)=>languages.find(l=>l.code===code)?.label??code;
 const available=languages.filter(l=>!value?.targets.includes(l.code));
 const remove=(code:string)=>{if(!value||value.targets.length<=1)return;const targets=value.targets.filter(t=>t!==code);store.update({targets,activeTarget:value.activeTarget===code?targets[0]!:value.activeTarget});};
 return <SettingsSection title="口述翻译">
  {value?.targets.map(code=><div key={code} className={s.translationRow} data-translation-language={code}><SettingsRow title={label(code)} description={code===value.activeTarget?'上次选择':undefined}><div className={s.translationActions}>
   <IconButton label={`移除${label(code)}`} disabled={!state.ready||state.busy||value.targets.length<=1} onClick={()=>remove(code)}><Icon name="x" size={16}/></IconButton>
  </div></SettingsRow></div>)}
  {value&&<SettingsRow title="添加语言"><Select value="" label="添加翻译语言" disabled={!state.ready||state.busy||!available.length} options={available.map(l=>({value:l.code,label:l.label}))} onValueChange={code=>store.update({targets:[...value.targets,code]})}/></SettingsRow>}
  {(error||state.error)&&<><InlineMessage status="error">{error||state.error}</InlineMessage><Button disabled={state.busy} onClick={()=>{void store.load();setReload(n=>n+1);}}>重新读取</Button></>}
 </SettingsSection>;
}
