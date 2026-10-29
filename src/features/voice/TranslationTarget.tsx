import {useEffect,useRef,useState} from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import type {TranslationLanguage,TranslationPort} from '../../contracts/translation';
import type {DictationSnapshot} from '../../contracts/dictation';
import {SelectionHighlight,useSelectionHighlight} from '../../ui/selection';
import {ActionIcon} from '../../ui/ActionMenu';
import ui from '../../ui/ui.module.css';
import s from './voice.module.css';

export function TranslationTarget({snapshot,port}:{snapshot:DictationSnapshot;port:TranslationPort}) {
 const [languages,setLanguages]=useState<TranslationLanguage[]>([]),[pending,setPending]=useState(false),[error,setError]=useState('');
 const flight=useRef(false),alive=useRef(true);
 const {ref,active}=useSelectionHighlight('[role="menuitemradio"]');
 useEffect(()=>{alive.current=true;void port.read().then(c=>{if(alive.current)setLanguages(c.languages);}).catch(()=>{if(alive.current)setError('语言暂不可用');});return()=>{alive.current=false;};},[port]);
 const label=(code:string|null)=>languages.find(l=>l.code===code)?.label??code??'…';
 const menu=(open:boolean)=>{if(!snapshot.taskId)return;void port.menu(snapshot.taskId,open).catch(()=>{if(alive.current)setError('语言选择已结束');});};
 const select=async(code:string)=>{if(flight.current||!snapshot.taskId||!snapshot.translationSelectable)return;flight.current=true;setPending(true);setError('');try{await port.select(snapshot.taskId,code);}catch{if(alive.current)setError('未切换，请重试');}finally{flight.current=false;if(alive.current)setPending(false);}};
 return <div className={s.translationTarget}>
  <Menu.Root modal={false} open={snapshot.translationMenu&&snapshot.translationSelectable} onOpenChange={menu}>
   <Menu.Trigger asChild><button type="button" className={s.languageTrigger} disabled={!snapshot.translationSelectable||pending||!languages.length} aria-label="翻译目标语言" aria-busy={pending}><span>翻译为</span><strong>{label(snapshot.targetLanguage)}</strong><ActionIcon name="expand" size={14}/></button></Menu.Trigger>
   <Menu.Portal><Menu.Content ref={ref} className={`${ui.actionMenu} ${s.languageMenu}`} data-motion={active?'active':'static'} align="center" side="top" sideOffset={8} collisionPadding={12} onCloseAutoFocus={e=>e.preventDefault()} onFocusOutside={e=>e.preventDefault()}>
    <SelectionHighlight/><Menu.RadioGroup value={snapshot.targetLanguage??''} onValueChange={code=>void select(code)}>{snapshot.translationTargets.map(code=><Menu.RadioItem key={code} value={code} disabled={pending||!snapshot.translationSelectable} className={ui.menuItem}>{label(code)}<Menu.ItemIndicator aria-hidden="true" className={s.languageCheck}>✓</Menu.ItemIndicator></Menu.RadioItem>)}</Menu.RadioGroup>
   </Menu.Content></Menu.Portal>
  </Menu.Root>
  {error&&<span role="status" className={s.languageError}>{error}</span>}
 </div>;
}
