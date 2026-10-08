import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Button, Field, TextInput, PasswordInput, Select, Switch, SingleChoice, Tabs, InlineMessage, Keycap, VoiceControl } from '../ui';
import { zh } from '../locales/zh-CN';
import type { DemoAction, Snapshot, Surface, WindowAction } from '../contracts/runtime';
import s from './dev.module.css';
import { MotionExamples } from './MotionExamples';
export default function Catalogue({ snapshot, demo, windowAction, resync }: { snapshot: Snapshot; demo: (action: DemoAction)=>void; windowAction: (surface: Surface,action: WindowAction)=>void; resync: ()=>void }) {
  const [value,setValue]=useState(''); const [select,setSelect]=useState('one'); const [checked,setChecked]=useState(false); const [tab,setTab]=useState('base');const [choice,setChoice]=useState('toggle');
  const t=zh.dev;
  return <section className={s.catalogue}><h1>{t.title}</h1><p>{t.note}</p><Tabs label={t.title} value={tab} onValueChange={setTab} options={[{value:'base',label:t.tabs[0]},{value:'feedback',label:t.tabs[1]}]}><div className={s.group}>
    <div className={s.row}><Button variant="primary">{t.primary}</Button><Button>{t.secondary}</Button><Button disabled>{t.disabled}</Button><Button loading>{t.loading}</Button><Keycap>Ctrl</Keycap></div>
    <Field label={t.label} description={t.description} error={value==='!' ? t.error : undefined}>{props=><TextInput {...props} value={value} placeholder={t.inputPlaceholder} onChange={e=>setValue(e.target.value)}/>}</Field>
    <Field label={t.password}>{props=><PasswordInput {...props} defaultValue="demo-only"/>}</Field>
    <div className={s.row}><Select label={t.select} value={select} onValueChange={setSelect} options={[{value:'one',label:'默认麦克风'},{value:'two',label:'长标签：会议室外接麦克风，保留完整名称并验证多行高亮边界'},{value:'three',label:'外接设备'}]}/><Switch label={t.enabled} checked={checked} onCheckedChange={setChecked}/></div>
    <div className={s.row}><SingleChoice label="口述方式示例" value={choice} onValueChange={setChoice} options={[{value:'toggle',label:'点按'},{value:'hold',label:'按住'}]}/><SingleChoice label="禁用单选示例" value="toggle" disabled onValueChange={()=>{}} options={[{value:'toggle',label:'点按'},{value:'hold',label:'按住'}]}/><Switch label="禁用开关示例" checked disabled onCheckedChange={()=>{}}/></div>
    <InlineMessage status="success">{t.success}</InlineMessage>
    <MotionExamples/>
  </div></Tabs><h2>{t.session}</h2><output data-testid="runtime-snapshot">{JSON.stringify(snapshot)}</output><div className={s.row}><Button onClick={()=>windowAction('overlay','show')}>{t.overlay}</Button><Button onClick={()=>windowAction('overlay','hide')}>{t.hideOverlay}</Button><Button onClick={()=>demo('fail')}>{t.fail}</Button><Button onClick={resync}>{t.reload}</Button></div>
    <div className={s.row}><VoiceControl motionEnabled={snapshot.windows.main} phase={snapshot.session.phase} onStart={()=>demo('start')} onFinish={()=>demo('finish')} onCancel={()=>demo('cancel')}/></div>
    <Link to="/onboarding">{t.introduction} →</Link><Link to="/">{zh.placeholder.back} →</Link>
  </section>;
}
