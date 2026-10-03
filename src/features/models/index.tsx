import {useCallback,useEffect,useRef,useState} from 'react';
import {Button,Field,TextInput,PasswordInput,Select,InlineMessage,AsyncStatusFeedback} from '../../ui';
import {modelError,modelExpectation,modelSameOrigin,type ModelConfigPort,type ModelOverview,type ModelInput,type ModelRecord,type ModelDraft,type ModelExpectation,type ModelTestResult,type ModelErrorCode} from '../../contracts/models';
import s from './models.module.css';
const empty=():ModelInput=>({name:'',protocol:'openai-chat',baseUrl:'',modelId:'',usages:['text']});
const errors:Record<ModelErrorCode,string>={
 invalid:'请检查输入内容及长度。',notFound:'配置已不存在，请重新加载列表。',conflict:'配置或默认选择已变化，请重新加载后确认。',
 busy:'另一个操作正在进行，请稍后再试。',unavailable:'本地数据暂时不可用，请重试。',preparing:'配置管理正在准备，请重试。',stopping:'应用正在退出。',
 credentialMissing:'密钥缺失，请重新填写。',credentialUnavailable:'无法访问系统凭据存储，请重试。',originChanged:'服务主机或端口已变化，请重新填写密钥。',
 forbidden:'当前窗口无权访问配置。',auth:'服务拒绝认证，请检查密钥与权限。',rateLimit:'服务限流，请稍后再试。',
 timeout:'请求超时，请检查服务地址或稍后重试。',network:'无法连接服务，请检查网络及证书。',protocol:'服务响应与兼容协议不符，可手动填写模型 ID。',
 cancelled:'请求已取消。',responseTooLarge:'服务响应超过大小限制。',emptyText:'请求已接受，但没有返回可见文字。',service:'服务暂时不可用，请稍后再试。',
};
const usageLabel=(usage:string)=>usage==='speech'?'语音识别':'文字处理';
interface Editor {input:ModelInput;key:string;record:ModelRecord|null}
const fromRecord=(c:ModelRecord):Editor=>({input:{name:c.name,protocol:c.protocol,baseUrl:c.baseUrl,modelId:c.modelId,usages:[...c.usages]},key:'',record:c});
export interface ModelPanelProps {port:ModelConfigPort;onGuardChange?:(guard:((action:()=>void)=>void)|null)=>void}
export function ModelPanel({port,onGuardChange}:ModelPanelProps){
 const [view,setView]=useState<ModelOverview|null>(null);
 const [editor,setEditor]=useState<Editor|null>(null);
 const [loading,setLoading]=useState(true),[saving,setSaving]=useState(false);
 const [error,setError]=useState<{text:string;field?:string;conflict?:boolean}|null>(null);
 const [notice,setNotice]=useState('');
 const [network,setNetwork]=useState<'list'|'test'|null>(null);
 const [models,setModels]=useState<string[]>([]);
 const [tested,setTested]=useState<ModelTestResult|null>(null);
 const [networkError,setNetworkError]=useState<string|null>(null);
 const [confirmation,setConfirmation]=useState<{kind:'remove'|'save';record:ModelRecord;expectation:ModelExpectation}|null>(null);
 const [leaving,setLeaving]=useState(false);
 const nextAction=useRef<(()=>void)|null>(null);
 const generation=useRef(0),loadVersion=useRef(0),request=useRef<string|null>(null),alive=useRef(false);
 const confirmFocus=useRef<HTMLDivElement|null>(null);
 const changed=!!editor&&(editor.key!==''||JSON.stringify(editor.input)!==JSON.stringify(editor.record?fromRecord(editor.record).input:empty()));
 const report=useCallback((e:unknown)=>{const value=modelError(e);setError({text:errors[value.code],field:value.field,conflict:value.code==='conflict'||value.code==='notFound'});},[]);
 const cancel=useCallback(()=>{
  generation.current++;const id=request.current;request.current=null;if(id)void port.cancelRequest(id).catch(()=>{});
  setNetwork(null);
 },[port]);
 const load=useCallback(async(retry=false)=>{
  const version=++loadVersion.current;
  setLoading(true);setError(null);
  try {
   const status=await port.status(retry);
   if(status.state!=='ready')throw {code:status.state==='preparing'?'preparing':'unavailable'};
   const overview=await port.overview();
   if(!alive.current||loadVersion.current!==version)return;
   setView(overview);if(!overview.configs.length)setEditor({input:empty(),key:'',record:null});
  }catch(e){if(alive.current&&loadVersion.current===version)report(e);}finally{if(alive.current&&loadVersion.current===version)setLoading(false);}
 },[port,report]);
 useEffect(()=>{let stopped=false;const counter=generation;alive.current=true;void Promise.resolve().then(()=>{if(!stopped)void load();});return()=>{stopped=true;alive.current=false;counter.current++;const id=request.current;if(id)void port.cancelRequest(id).catch(()=>{});request.current=null;};},[port,load]);
 const guard=useCallback((action:()=>void)=>{
  if(saving){setNotice('正在保存，请等待完成后再切换。');return;}
  cancel();
  if(changed){setConfirmation(null);nextAction.current=action;setLeaving(true);}else{setEditor(null);action();}
 },[saving,cancel,changed]);
 useEffect(()=>{onGuardChange?.(guard);return()=>onGuardChange?.(null);},[guard,onGuardChange]);
 useEffect(()=>{if(confirmation||leaving)confirmFocus.current?.querySelector<HTMLButtonElement>('button')?.focus();},[confirmation,leaving]);
 function modify(patch:Partial<ModelInput>,key?:string){
  cancel();setTested(null);if(patch.baseUrl!==undefined||key!==undefined)setModels([]);setNetworkError(null);setConfirmation(null);setError(null);setNotice('');
  setEditor(current=>current?{...current,input:{...current.input,...patch},key:key??current.key}:current);
 }
 function draft():ModelDraft {
  if(!editor)throw new Error('No editor');
  return {input:structuredClone(editor.input),key:editor.key.trim()?{action:'replace',value:editor.key}:{action:'keep'},id:editor.record?.id??null,revision:editor.record?.revision??null};
 }
 async function check(kind:'list'|'test'){
  cancel();setTested(null);setNetworkError(null);setNotice('');
  const id=crypto.randomUUID(),version=generation.current;request.current=id;setNetwork(kind);
  try {
   if(kind==='list'){
    const ids=await port.fetchModels(draft(),id);
    if(alive.current&&generation.current===version){setModels(ids);setNotice(ids.length?'列表已获取；选择或手动填写模型 ID。':'服务列表为空，仍可手动填写模型 ID。');}
   }else{
    const result=await port.testTextModel(draft(),id);
    if(alive.current&&generation.current===version)setTested(result);
   }
  }catch(e){if(alive.current&&generation.current===version)setNetworkError(errors[modelError(e).code]);}
  finally {if(alive.current&&generation.current===version){request.current=null;setNetwork(null);}}
 }
 async function save(confirm=false){
  if(!editor||!view||saving)return;
  const expected=editor.record?modelExpectation(view,editor.record.id,editor.input.usages):null;
  if(!confirm&&expected?.replacements.length){setConfirmation({kind:'save',record:editor.record!,expectation:expected});return;}
  cancel();setSaving(true);setError(null);setNotice('');
  try {
   const result=await port.save({...draft(),expectation:confirm&&confirmation?confirmation.expectation:expected});
   if(!alive.current)return;setView(result);setEditor(null);setConfirmation(null);setModels([]);setTested(null);
   setNotice(result.cleanupPending?'配置已保存；旧凭据尚待清理，可重试清理。':'配置已保存。');
  }catch(e){if(alive.current)report(e);}finally{if(alive.current)setSaving(false);}
 }
 async function remove(){
  if(!confirmation||confirmation.kind!=='remove'||saving)return;
  cancel();setSaving(true);setError(null);
  try{
   const result=await port.remove({id:confirmation.record.id,revision:confirmation.record.revision,expectation:confirmation.expectation});
   if(!alive.current)return;setView(result);setEditor(result.configs.length?null:{input:empty(),key:'',record:null});setConfirmation(null);
   setNotice(result.cleanupPending?'配置已删除；旧凭据尚待清理。':'配置已删除。');
  }catch(e){if(alive.current){setConfirmation(null);report(e);}}finally{if(alive.current)setSaving(false);}
 }
 async function setDefault(usage:'speech'|'text',id:string){
  if(!view||saving)return;
  setSaving(true);setError(null);
  try{const d=view.defaults.find(d=>d.usage===usage)!;const result=await port.setDefault({usage,configId:id,revision:d.revision});if(alive.current){setView(result);setNotice('默认配置已更新。');}}
  catch(e){if(alive.current)report(e);}finally{if(alive.current)setSaving(false);}
 }
 function open(record:ModelRecord|null){
  guard(()=>{setEditor(record?fromRecord(record):{input:empty(),key:'',record:null});setError(null);setNotice('');setTested(null);setModels([]);setConfirmation(null);setNetworkError(null);});
 }
 function discard(){cancel();setEditor(null);setLeaving(false);setConfirmation(null);const action=nextAction.current;nextAction.current=null;action?.();}
 const originChanged=!!editor?.record&&!modelSameOrigin(editor.record.baseUrl,editor.input.baseUrl);
 const fieldError=(key:string)=>error?.field===key?error.text:undefined;
 const editorForm=editor&&<form className={s.form} onSubmit={e=>{e.preventDefault();void save();}}>
    <h3>{editor.record?'编辑配置':'添加 API 模型'}</h3>
    <fieldset disabled={saving} className={s.fields}>
     <Field label="配置名称" error={fieldError('name')}>{props=><TextInput {...props} value={editor.input.name} maxLength={128} autoComplete="off" onChange={e=>modify({name:e.target.value})}/>}</Field>
     <Field label="接口类型">{props=><Select {...props} label="接口类型" value="openai-chat" onValueChange={()=>{}} options={[{value:'openai-chat',label:'OpenAI 兼容 · Chat Completions'}]}/>}</Field>
     <Field label="服务基址" error={fieldError('baseUrl')} description="填写完整 HTTPS 基址，例如 https://api.openai.com/v1。">{props=><TextInput {...props} value={editor.input.baseUrl} placeholder="https://api.openai.com/v1" autoComplete="off" spellCheck={false} onChange={e=>modify({baseUrl:e.target.value})}/>}</Field>
     <Field label="API Key" error={fieldError('key')} description={originChanged?'服务源站已改变，必须重新填写密钥。':editor.record?'留空保留已保存的密钥；显隐只显示本次输入。':'密钥保存在 Windows 凭据存储，不写入数据库。'}>{props=><PasswordInput {...props} value={editor.key} placeholder={editor.record?.credentialStatus==='present'?'密钥已保存':''} autoComplete="off" onChange={e=>modify({},e.target.value)}/>}</Field>
     <div className={s.tools}><Button loading={network==='list'} disabled={network!==null&&network!=='list'} onClick={()=>void check('list')}>获取模型列表</Button></div>
     <Field label="模型 ID" error={fieldError('modelId')} description="列表获取失败时可手动填写，不影响保存。">{props=><TextInput {...props} list="sayweave-model-ids" value={editor.input.modelId} autoComplete="off" spellCheck={false} onChange={e=>modify({modelId:e.target.value})}/>}</Field>
     <datalist id="sayweave-model-ids">{models.map(id=><option value={id} key={id}/>)}</datalist>
     <div className={s.usages}><span>用途</span>{(['speech','text'] as const).map(u=><label key={u}><input type="checkbox" checked={editor.input.usages.includes(u)} onChange={e=>modify({usages:e.target.checked?[...editor.input.usages,u]:editor.input.usages.filter(v=>v!==u)})}/>{usageLabel(u)}</label>)}</div>
     {fieldError('usages')&&<InlineMessage status="error">{fieldError('usages')}</InlineMessage>}
     {editor.input.usages.includes('speech')&&<p className={s.meta}>语音用途可保存和设为默认，识别能力尚未验证；本轮不发送音频。</p>}
     <p className={s.meta}>{port.mode==='native'?'测试会发送一次短请求，可能产生少量费用。保存不会自动测试。':'测试仅演示状态反馈，不代表服务可用。'}</p>
     <div className={s.tools}><Button disabled={!editor.input.usages.includes('text')||(network!==null&&network!=='test')} loading={network==='test'} onClick={()=>void check('test')}>测试文字模型</Button>{network&&<Button variant="quiet" onClick={()=>{cancel();setNetworkError('请求已取消。');}}>取消请求</Button>}</div>
     <AsyncStatusFeedback status={network?'pending':networkError?'error':tested?'success':'idle'} message={network?(network==='list'?'正在获取列表':'正在测试文字模型'):networkError??(tested?(port.mode==='native'?'本次文字测试通过':'模拟测试完成')+' · '+tested.elapsedMs+' ms':'尚未测试')}/>
    </fieldset>
    <div className={s.actions}><Button variant="primary" loading={saving} onClick={()=>void save()}>保存配置</Button><Button disabled={saving} onClick={()=>guard(()=>setEditor(null))}>取消编辑</Button></div>
   </form>;
 return <div className={s.panel}>
  <p className={s.intro}>保存个人 API 配置，分别指定语音识别与文字处理的默认模型。</p>
  {port.mode==='simulation'&&<InlineMessage>浏览器模拟：配置仅存在内存中，测试不发请求、不访问系统凭据。</InlineMessage>}
  {loading&&<p role="status">正在读取配置…</p>}
  {error&&<><InlineMessage status="error">{error.text}</InlineMessage>{error.conflict&&<Button disabled={saving} onClick={()=>guard(()=>{setEditor(null);void load(true);})}>重新加载已保存配置</Button>}</>}
  {!view&&!loading&&<Button onClick={()=>void load(true)}>重试读取配置</Button>}
  {view&&<>
   <div className={s.defaults}>
    {view.defaults.map(d=><Field key={d.usage} label={usageLabel(d.usage)+'默认配置'} description={d.usage==='speech'?'用途由你声明，识别能力尚未验证。':undefined}>{props=><select {...props} className={s.select} value={d.configId??''} disabled={saving||!view.configs.some(c=>c.usages.includes(d.usage))} onChange={e=>void setDefault(d.usage,e.target.value)}><option value="" disabled>尚未选择默认配置</option>{view.configs.filter(c=>c.usages.includes(d.usage)).map(c=><option key={c.id} value={c.id}>{c.name} · {c.modelId}</option>)}</select>}</Field>)}
   </div>
   {view.cleanupPending&&<InlineMessage>配置可用，但凭据清理尚未完成。<Button disabled={saving} onClick={()=>void load(true)}>重试清理</Button></InlineMessage>}
   <div className={s.listHeader}><h3>已保存配置 <span>{view.configs.length}</span></h3><Button disabled={saving} onClick={()=>open(null)}>添加模型</Button></div>
   <div className={s.list}>{view.configs.map(c=><div key={c.id}><article className={s.summary}>
    <button className={s.summaryButton} aria-expanded={editor?.record?.id===c.id} disabled={saving} onClick={()=>open(c)}><strong>{c.name}</strong><span>{c.modelId} · {new URL(c.baseUrl).host}</span><small>{c.usages.map(usageLabel).join(' / ')}{view.defaults.filter(d=>d.configId===c.id).map(d=>' · '+usageLabel(d.usage)+'默认')}{c.credentialStatus!=='present'?' · 密钥'+(c.credentialStatus==='missing'?'缺失':'不可用'):''}</small></button>
    <Button variant="quiet" disabled={saving} onClick={()=>guard(()=>{setConfirmation({kind:'remove',record:c,expectation:modelExpectation(view,c.id,[])});setError(null);})}>删除</Button>
   </article>{editor?.record?.id===c.id&&editorForm}</div>)}</div>
   {editor&&!editor.record&&editorForm}
   {confirmation&&<div ref={confirmFocus} className={s.confirmation} role="alertdialog" aria-label={confirmation.kind==='remove'?'删除配置确认':'用途变更确认'}>
    <h3>{confirmation.kind==='remove'?'删除“'+confirmation.record.name+'”？':'保存用途变更？'}</h3>
    {confirmation.expectation.replacements.length?confirmation.expectation.replacements.map(r=>{const target=view.configs.find(c=>c.id===r.configId);return <p key={r.usage}>{usageLabel(r.usage)}默认将{target?<>改为 <strong>{target.name}</strong> · {target.modelId}<br/>{target.baseUrl}</>:'清空，尚未选择默认配置。'}</p>;}):<p>其他默认选择保持不变。</p>}
    <div className={s.actions}><Button variant="primary" loading={saving} onClick={()=>void(confirmation.kind==='remove'?remove():save(true))}>{confirmation.kind==='remove'?'确认删除':'确认保存'}</Button><Button disabled={saving} onClick={()=>setConfirmation(null)}>取消</Button></div>
   </div>}
  </>}
  {leaving&&<div ref={confirmFocus} className={s.confirmation} role="alertdialog" aria-label="未保存修改"><h3>有未保存的修改</h3><p>继续编辑，或放弃本次修改。</p><div className={s.actions}><Button variant="primary" onClick={()=>{setLeaving(false);nextAction.current=null;}}>继续编辑</Button><Button onClick={discard}>放弃修改</Button></div></div>}
  {notice&&<InlineMessage>{notice}</InlineMessage>}
  <p className={s.meta}>本地模型的下载、导入和运行尚未实现。</p>
 </div>;
}

