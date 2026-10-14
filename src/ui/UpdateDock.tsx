import {useSyncExternalStore} from 'react';
import type {UpdatePort} from '../contracts/update';
import {Icon} from './Icon';
import {UpdateProgress} from './UpdateProgress';
import {updateLabel} from './update-state';
import s from './update-dock.module.css';

export function UpdateDock({port,onAbout}:{port:UpdatePort;onAbout:()=>void}) {
 const value=useSyncExternalStore(port.subscribe,port.snapshot);
 const active=value.state!=='idle'||value.recovery;
 const label=active?`${updateLabel(value.state,value.version,value.progress)}，打开关于`:'关于';
 return <button type="button" className={s.trigger} aria-label={label} title={label} onClick={onAbout}>
  {active?<UpdateProgress value={value}/>:<Icon name="info" size={18}/>}
 </button>;
}
