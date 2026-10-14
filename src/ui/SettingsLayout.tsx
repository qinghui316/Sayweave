import {useId,type ReactNode} from 'react';
import s from './settings-layout.module.css';

export function SettingsSection({title,children}:{title:string;children:ReactNode}){
 const id=useId();
 return <section className={s.section} aria-labelledby={id}><h3 id={id} className={s.sectionTitle}>{title}</h3><div className={s.rows}>{children}</div></section>;
}

/** Callers keep control labels, state and actions; this component only owns layout. */
export function SettingsRow({title,description,children,feedback}:{title:ReactNode;description?:ReactNode;children:ReactNode;feedback?:ReactNode}){
 return <div className={s.row} data-settings-row=""><div className={s.copy}><div className={s.title}>{title}</div>{description&&<p className={s.description}>{description}</p>}</div><div className={s.control}>{children}</div>{feedback&&<div className={s.feedback}>{feedback}</div>}</div>;
}
