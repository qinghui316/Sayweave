import {useEffect,useRef,useState} from 'react';
import {Button,Dialog,InlineMessage} from '../../ui';
import {componentMessage,type TextServicePort,type TextServiceAction,type TextServiceStatus} from '../../contracts/text-service';
import s from './settings.module.css';
export function TextServiceSettings({port}:{port:TextServicePort}){
 const [status,setStatus]=useState<TextServiceStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[confirm,setConfirm]=useState(false);
 const generation=useRef({value:0});
 const deploymentBlocked=status?.deploymentState==='pending'||status?.deploymentState==='failed';
 useEffect(()=>{const epoch=generation.current;const id=++epoch.value;void port.status().then(v=>{if(id===epoch.value)setStatus(v);}).catch(()=>{if(id===epoch.value)setError(componentMessage('componentUnavailable'));});return()=>{epoch.value++;};},[port]);
 async function act(action?:TextServiceAction){if(busy)return;const id=++generation.current.value;setBusy(true);setError('');setNotice('');try{const v=action?await port.act(action):await port.status();if(id===generation.current.value){setStatus(v);if(action==='install')setNotice('组件已就绪。更新后请重新打开正在使用的输入软件。');if(action==='remove')setNotice('原位输入组件已移除。重新打开输入软件后生效。');}}catch(e){if(id===generation.current.value)setError(componentMessage(typeof e==='object'&&e&&'code'in e?String(e.code):'componentUnavailable'));}finally{if(id===generation.current.value)setBusy(false);}}
 return <section><div className={s.settingRow}><div><h3>原位输入</h3><p>{port.mode==='simulation'?'浏览器仅预览组件设置。':status?.enabled?'已启用，说话时文字会直接出现在输入框。':'启用后，文字会随口述直接出现在输入框。'}</p></div>{status&&<Button disabled={busy||deploymentBlocked||(!status.installed&&!status.payloadAvailable)} onClick={()=>void act(status.enabled?undefined:status.installed?'enable':'install')}>{busy?'处理中…':status.enabled?'检查状态':status.installed?'启用':'安装并启用'}</Button>}</div>
 {status?.enabled&&<Button variant="quiet" disabled={busy||deploymentBlocked} onClick={()=>void act('disable')}>停用</Button>}
 {status?.deploymentState==='pending'&&<p role="status">正在准备原位输入，请稍候。</p>}
 {status?.deploymentState==='failed'&&<InlineMessage status="error">输入组件维护未完成，请重新运行安装包恢复。</InlineMessage>}
 {status&&!status.installed&&port.mode==='native'&&<p>首次安装需要 Windows 管理员确认。</p>}
 {(status?.installed||status?.cleanupPending)&&<details><summary>组件管理</summary><p>{status.cleanupPending?'文件仍被输入软件使用。关闭该软件后可重试清理。':'修复或移除需要 Windows 管理员确认。'}</p>{status.updateAvailable&&<Button variant="quiet" disabled={busy||deploymentBlocked} onClick={()=>void act('install')}>修复组件</Button>}<Button variant="quiet" disabled={busy||deploymentBlocked} onClick={()=>setConfirm(true)}>{status.cleanupPending?'重试清理':'移除组件'}</Button></details>}
 {status?.error&&<InlineMessage status="error">{componentMessage(status.error)}</InlineMessage>}{busy&&<p role="status">正在处理输入组件…</p>}{notice&&<p role="status">{notice}</p>}{error&&<InlineMessage status="error">{error}<Button variant="quiet" disabled={busy} onClick={()=>void act()}>重新检查</Button></InlineMessage>}
 <Dialog confirmation open={confirm} onOpenChange={setConfirm} title="移除原位输入组件" description="移除后可以在这里重新安装。Windows 将请求管理员确认。"><Button autoFocus variant="quiet" onClick={()=>setConfirm(false)}>取消</Button><Button onClick={()=>{setConfirm(false);void act('remove');}}>移除</Button></Dialog></section>;
}
