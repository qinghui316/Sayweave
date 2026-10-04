import {createElement} from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import {Ellipsis,Play,Pause,ChevronDown,X,RotateCw} from 'lucide';
import {IconButton} from './controls';
import s from './ui.module.css';
const shapes={more:Ellipsis,play:Play,pause:Pause,expand:ChevronDown,close:X,retry:RotateCw};
export function ActionIcon({name,size=18}:{name:keyof typeof shapes;size?:number}) {
 return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{shapes[name].map(([tag,attrs],i)=>createElement(tag,{...attrs,key:i}))}</svg>;
}
export function ActionMenu({label='更多操作',items,onOpenChange}:{label?:string;items:{label:string;disabled?:boolean;onSelect:()=>void}[];onOpenChange?:(open:boolean)=>void}) {
 return <Menu.Root modal={false} onOpenChange={onOpenChange}><Menu.Trigger asChild><IconButton label={label}><ActionIcon name="more"/></IconButton></Menu.Trigger><Menu.Portal><Menu.Content className={s.actionMenu} align="end" sideOffset={6} collisionPadding={12}>{items.map(item=><Menu.Item key={item.label} className={s.menuItem} disabled={item.disabled} onSelect={item.onSelect}>{item.label}</Menu.Item>)}</Menu.Content></Menu.Portal></Menu.Root>;
}
