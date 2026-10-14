import type {UpdateSnapshot} from '../contracts/update';
import s from './update-progress.module.css';

/** Sidebar state visual; only native verification makes it ready. */
export function UpdateProgress({value}:{value:Pick<UpdateSnapshot,'state'|'progress'|'recovery'>}) {
 const failed=value.state==='failed'||value.state==='recovery'||value.recovery;
 const ready=value.state==='readyToInstall'&&!failed;
 const progress=value.state==='downloading'&&value.progress!==null&&Number.isFinite(value.progress)?Math.max(0,Math.min(100,value.progress)):null;
 return <svg viewBox="0 0 24 24" className={s.graphic} aria-hidden="true" data-update-visual={failed?'error':ready?'download':progress===null?'indeterminate':'progress'}>
  {failed?<g className={s.error}><circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 3h.01"/></g>:ready?<path className={s.download} d="M12 3v12m-4-4 4 4 4-4M4 17v3h16v-3"/>:<>
   <circle className={s.track} cx="12" cy="12" r="9"/>
   <circle className={`${s.progress} ${progress===null?s.spin:''}`} cx="12" cy="12" r="9" pathLength="100" strokeDasharray="100" strokeDashoffset={progress===null?75:100-progress} style={progress===0?{visibility:'hidden'}:undefined}/>
  </>}
 </svg>;
}
