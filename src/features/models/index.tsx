import {useCallback, useEffect, useId, useRef, useState} from 'react';
import {Button, Dialog, Field, TextInput, PasswordInput, Select, InlineMessage, AsyncStatusFeedback, Tabs} from '../../ui';
import {modelError, modelExecutable, modelExpectation, modelSameOrigin, type ModelConfigPort, type ModelOverview, type ModelInput, type ModelRecord, type ModelDraft, type ModelExpectation, type ModelTestResult, type ModelErrorCode, type ModelUsage} from '../../contracts/models';
import {initialModel, modelName, speechPresets} from './presets';
import s from './models.module.css';

const errors: Record<ModelErrorCode, string> = {
 invalid:'请检查填写内容。', notFound:'这个模型已被删除，请重新加载。', conflict:'模型或当前选择已变化，请重新加载后确认。',
 busy:'另一个操作正在进行，请稍后再试。', unavailable:'暂时无法读取模型，请重试。', preparing:'正在准备模型，请稍后重试。', stopping:'应用正在退出。',
 credentialMissing:'请填写 API 密钥。', credentialUnavailable:'无法读取或保存密钥，请重试。', originChanged:'服务地址已改变，请重新填写 API 密钥。',
 forbidden:'当前窗口无法管理模型。', auth:'密钥验证失败，请检查 API 密钥。', rateLimit:'请求过于频繁，请稍后再试。',
 timeout:'连接超时，请检查地址或稍后重试。', network:'无法连接服务，请检查网络和地址。', protocol:'无法获取有效结果，可以手动填写模型。',
 cancelled:'请求已取消。', responseTooLarge:'返回内容过大，请换一个服务后重试。', emptyText:'服务没有返回文字，请检查所选模型。', service:'服务暂时不可用，请稍后再试。',
};
const label = (usage: ModelUsage) => usage === 'speech' ? '语音识别' : '文字处理';
const recordsFor = (view: ModelOverview, usage: ModelUsage) => view.configs.filter(c => usage === 'text' ? c.protocol === 'openai-chat' : modelExecutable(c, usage));
interface Editor {input:ModelInput; initial:ModelInput; key:string; record:ModelRecord|null; automaticName:boolean}
function newEditor(usage:ModelUsage):Editor {
 const input = initialModel(usage);
 return {input, initial:structuredClone(input), key:'', record:null, automaticName:true};
}
function fromRecord(c:ModelRecord):Editor {
 const input = {name:c.name, protocol:c.protocol, baseUrl:c.baseUrl, modelId:c.modelId, usages:[...c.usages]};
 return {input, initial:structuredClone(input), key:'', record:c, automaticName:false};
}
export interface ModelPanelProps {port:ModelConfigPort; onSpeech?:()=>void; onChanged?:()=>void; onHelp?:(help:import('../../contracts/models').ModelHelp)=>Promise<void>; onGuardChange?:(guard:((action:()=>void)=>void)|null)=>void}
export function ModelPanel({port, onSpeech, onHelp, onGuardChange, onChanged}: ModelPanelProps) {
 const [usage, setUsage] = useState<ModelUsage>('speech');
 const [view, setView] = useState<ModelOverview|null>(null);
 const [editor, setEditor] = useState<Editor|null>(null);
 const [advanced, setAdvanced] = useState(false);
 const [loading, setLoading] = useState(true), [saving, setSaving] = useState(false);
 const [error, setError] = useState<{text:string; field?:string; conflict?:boolean}|null>(null);
 const [notice, setNotice] = useState('');
 const [network, setNetwork] = useState<'test'|null>(null);
 const [tested, setTested] = useState<ModelTestResult|null>(null);
 const [networkError, setNetworkError] = useState<string|null>(null);
 const [confirmation, setConfirmation] = useState<{record:ModelRecord; expectation:ModelExpectation}|null>(null);
 const [leaving, setLeaving] = useState(false);
 const nextAction = useRef<(()=>void)|null>(null);
 const generation = useRef(0), loadVersion = useRef(0), request = useRef<string|null>(null), alive = useRef(false);
 const form = useRef<HTMLFormElement|null>(null);
 const formId = useId();
 const changed = !!editor && (editor.key !== '' || JSON.stringify(editor.input) !== JSON.stringify(editor.initial));
 const report = useCallback((e:unknown, category?:ModelUsage) => {
  const value = modelError(e);
  if(value.field === 'name' || (value.field === 'baseUrl' && category === 'speech')) setAdvanced(true);
  setError({text:errors[value.code], field:value.field ?? (value.code === 'credentialMissing' || value.code === 'originChanged' ? 'key' : undefined), conflict:value.code === 'conflict' || value.code === 'notFound'});
 }, []);
 const cancel = useCallback(() => {
  generation.current++;
  const id = request.current; request.current = null;
  if(id) void port.cancelRequest(id).catch(() => {});
  setNetwork(null);
 }, [port]);
 const load = useCallback(async(retry = false) => {
  const version = ++loadVersion.current;
  setLoading(true); setError(null);
  try {
   const status = await port.status(retry);
   if(status.state !== 'ready') throw {code:status.state === 'preparing' ? 'preparing' : 'unavailable'};
   const overview = await port.overview();
   if(!alive.current || loadVersion.current !== version) return;
   setView(overview);
   setEditor(recordsFor(overview, 'speech').length ? null : newEditor('speech'));
   setUsage('speech'); setAdvanced(false);
  } catch(e) {if(alive.current && loadVersion.current === version) report(e);}
  finally {if(alive.current && loadVersion.current === version) setLoading(false);}
 }, [port, report]);
 useEffect(() => {
  let stopped = false; alive.current = true;
  void Promise.resolve().then(() => {if(!stopped) void load();});
  const counter = generation;
  return () => {stopped = true; alive.current = false; counter.current++; const id = request.current; if(id) void port.cancelRequest(id).catch(() => {}); request.current = null;};
 }, [port, load]);
 const guard = useCallback((action:()=>void) => {
  if(saving) {setNotice('正在保存，请稍候。'); return;}
  cancel();
  if(changed) {setConfirmation(null); nextAction.current = action; setLeaving(true);}
  else {setEditor(null); action();}
 }, [saving, cancel, changed]);
 useEffect(() => {onGuardChange?.(guard); return () => onGuardChange?.(null);}, [guard, onGuardChange]);
 useEffect(() => {if(error?.field) form.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();}, [error, advanced]);
 function resetFeedback() {setError(null); setNotice(''); setTested(null); setNetworkError(null); setConfirmation(null); setAdvanced(false);}
 function switchUsage(value:string) {
  if(value === usage || !view) return;
  guard(() => {const next = value as ModelUsage; setUsage(next); setEditor(recordsFor(view, next).length ? null : newEditor(next)); resetFeedback();});
 }
 function open(record:ModelRecord|null) {guard(() => {setEditor(record ? fromRecord(record) : newEditor(usage)); resetFeedback();});}
 function modify(patch:Partial<ModelInput>, key?:string) {
  cancel(); setTested(null); setNetworkError(null); setError(null); setNotice('');
  setEditor(current => {
   if(!current) return current;
   const input = {...current.input, ...patch};
   if(patch.modelId !== undefined && current.automaticName) input.name = modelName(input.modelId, usage);
   return {...current, input, key:key ?? current.key, automaticName:patch.name === undefined && current.automaticName};
  });
 }
 function draft(): ModelDraft {
  if(!editor) throw new Error('No model draft');
  return {input:structuredClone(editor.input), key:editor.key.trim() ? {action:'replace', value:editor.key} : {action:'keep'}, id:editor.record?.id ?? null, revision:editor.record?.revision ?? null};
 }
 function selectSpeechPreset(modelId:string) {
  const preset=speechPresets.find(p=>p.modelId===modelId);
  if(!preset || !editor) return;
  const baseUrl=editor.input.protocol===preset.protocol?editor.input.baseUrl:preset.baseUrl;
  const changedService=editor.input.protocol!==preset.protocol || !modelSameOrigin(editor.input.baseUrl,baseUrl);
  modify({protocol:preset.protocol,baseUrl,modelId},changedService?'':undefined);
 }
 async function check() {
  if(!editor || network || saving) return;
  cancel(); setTested(null); setNetworkError(null); setNotice('');
  const id = crypto.randomUUID(), version = generation.current;
  request.current = id; setNetwork('test');
  try {
   const result = await port.testTextModel(draft(), id);
   if(alive.current && generation.current === version) setTested(result);
  } catch(e) {if(alive.current && generation.current === version) setNetworkError(errors[modelError(e).code]);}
  finally {if(alive.current && generation.current === version) {request.current = null; setNetwork(null);}}
 }
 async function save() {
  if(!editor || !view || saving) return;
  cancel(); setSaving(true); setError(null); setNotice('');
  try {
   const result = await port.save({...draft(), expectation:editor.record ? modelExpectation(view, editor.record.id, editor.input.usages) : null});
   if(!alive.current) return;
   setView(result); setEditor(null); setTested(null); onChanged?.();
   setNotice(result.cleanupPending ? '已保存，密钥清理稍后重试。' : '已保存。');
  } catch(e) {if(alive.current) report(e, usage);} finally {if(alive.current) setSaving(false);}
 }
 async function remove() {
  if(!confirmation || saving) return;
  setSaving(true); setError(null);
  try {
   const result = await port.remove({id:confirmation.record.id, revision:confirmation.record.revision, expectation:confirmation.expectation});
   if(!alive.current) return;
   setView(result); setEditor(recordsFor(result, usage).length ? null : newEditor(usage)); setConfirmation(null); onChanged?.();
   setNotice(result.cleanupPending ? '已删除，密钥清理稍后重试。' : '已删除。');
  } catch(e) {if(alive.current) {setConfirmation(null); report(e);}} finally {if(alive.current) setSaving(false);}
 }
 async function selectModel(id:string) {
  if(!view || saving) return;
  setSaving(true); setError(null); setNotice('');
  try {
   const current = view.defaults.find(d => d.usage === usage)!;
   const result = await port.setDefault({usage, configId:id, revision:current.revision});
   if(alive.current) {setView(result); setNotice('当前模型已更新。'); onChanged?.();}
  } catch(e) {if(alive.current) report(e);} finally {if(alive.current) setSaving(false);}
 }
 function continueEditing() {setLeaving(false); nextAction.current = null;}
 function discard() {cancel(); setEditor(null); setLeaving(false); const action = nextAction.current; nextAction.current = null; action?.();}
 const originChanged = !!editor?.record && (editor.record.protocol !== editor.input.protocol || !modelSameOrigin(editor.record.baseUrl, editor.input.baseUrl));
 const fieldError = (key:string) => error?.field === key ? error.text : undefined;
 const selected = view?.defaults.find(d => d.usage === usage)?.configId;
 const current = view?.configs.find(c => c.id === selected && modelExecutable(c, usage));
 const records = view ? recordsFor(view, usage) : [];
 const addressField = editor && <Field label="服务地址（URL）" error={fieldError('baseUrl')}>{props => <TextInput {...props} value={editor.input.baseUrl} autoComplete="off" spellCheck={false} onChange={e => modify({baseUrl:e.target.value})}/>}</Field>;
 const speechOptions = speechPresets.map(p => ({value:p.modelId as string, label:p.label as string}));
 if(editor && !speechOptions.some(p => p.value === editor.input.modelId)) speechOptions.unshift({value:editor.input.modelId, label:'当前自定义模型 · '+editor.input.modelId});
 return <Tabs variant="navigation" activationMode="manual" className={s.panel} contentClassName={s.tabContent} value={usage} onValueChange={switchUsage} disabled={saving || loading} label="模型类别" options={[{value:'speech', label:'语音识别'}, {value:'text', label:'文字处理'}]}>
  <div className={s.scroll} data-testid="model-scroll">
   {loading && <p role="status">正在读取模型…</p>}
   {!view && !loading && <Button onClick={() => void load(true)}>重新读取</Button>}
   {view?.cleanupPending && <InlineMessage>密钥清理尚未完成。<Button disabled={saving} onClick={() => guard(() => void load(true))}>重试清理</Button></InlineMessage>}
   {view && !editor && <>
    <div className={s.listHeader}><h3>已保存模型</h3><Button disabled={saving} onClick={() => open(null)}>添加模型</Button></div>
    {!current && <p className={s.meta}>请选择用于{label(usage)}的模型。</p>}
    <div className={s.list}>{[...records].sort((a,b) => Number(b.id === selected) - Number(a.id === selected)).map(c => <article className={s.summary} key={c.id}>
     <div className={s.summaryCopy}><div className={s.name}><strong>{c.name}</strong>{c.id === selected && <span className={s.current}>当前使用</span>}</div>{c.name!==c.modelId&&<p>{c.modelId}</p>}<small>{new URL(c.baseUrl).host}{c.credentialStatus !== 'present' ? ' · 请重新填写密钥' : ''}</small></div>
     {!modelExecutable(c, usage) && <p className={s.meta}>此前保存的语音配置，可以继续编辑或删除。实时识别请在“语音识别”中选择模型。</p>}
     <div className={s.rowActions}>{c.id !== selected && modelExecutable(c, usage) && <Button disabled={saving} onClick={() => void selectModel(c.id)}>使用此模型</Button>}<Button variant="quiet" disabled={saving} onClick={() => open(c)}>编辑</Button><Button variant="quiet" disabled={saving} onClick={() => guard(() => {setConfirmation({record:c, expectation:modelExpectation(view,c.id,[])}); setError(null);})}>删除</Button></div>
    </article>)}</div>
   </>}
   {editor && <form id={formId} ref={form} className={s.form} onSubmit={e => {e.preventDefault(); void save();}}>
    {usage === 'speech' && view?.configs.some(c => c.id === selected && !modelExecutable(c,'speech')) && <p className={s.meta}>此前的语音配置仍在“文字处理”中保留。选择模型并填写密钥后，即可试说。</p>}
    <fieldset disabled={saving} className={s.fields}>
     {usage === 'speech' ? <Field label="模型" error={fieldError('modelId')}>{props => <Select {...props} disabled={saving} label="模型" value={editor.input.modelId} onValueChange={selectSpeechPreset} options={speechOptions}/>}</Field> : addressField}
     <div className={s.keyGroup}>
      <Field label="API 密钥" error={fieldError('key')} description={originChanged ? '请填写这个服务的密钥。' : editor.record?.credentialStatus === 'present' ? '已保存；留空即可继续使用。' : undefined}>{props => <PasswordInput {...props} value={editor.key} placeholder={!originChanged && editor.record?.credentialStatus === 'present' ? '密钥已保存' : '粘贴 API 密钥'} autoComplete="off" onChange={e => modify({},e.target.value)}/>}</Field>
      {usage === 'speech' && <Button variant="quiet" disabled={saving} onClick={() => {if(onHelp) void onHelp(editor.input.protocol==='doubao-asr-streaming'?'doubao-api-key':'qwen-api-key').catch(() => setNotice('无法打开帮助，请稍后重试。'));}}>获取 API 密钥</Button>}
     </div>
     {usage === 'text' && <Field label="模型" error={fieldError('modelId')}>{props => <TextInput {...props} value={editor.input.modelId} placeholder="填写模型名称" autoComplete="off" spellCheck={false} onChange={e => modify({modelId:e.target.value})}/>}</Field>}
     <Button variant="quiet" className={s.more} aria-expanded={advanced} aria-controls={formId+'-advanced'} onClick={() => setAdvanced(value => !value)}>{advanced ? '收起设置' : '更多设置'}</Button>
     {advanced && <div id={formId+'-advanced'} className={s.advanced}>
      <Field label="名称" error={fieldError('name')}>{props => <TextInput {...props} value={editor.input.name} autoComplete="off" onChange={e => modify({name:e.target.value})}/>}</Field>
      {usage === 'speech' && addressField}
     </div>}
    </fieldset>
   </form>}
  </div>
  <footer className={s.footer}>
   {error && <div><InlineMessage status="error">{error.text}</InlineMessage>{error.conflict && <Button disabled={saving} onClick={() => guard(() => void load(true))}>重新加载</Button>}</div>}
   {notice && <InlineMessage>{notice}</InlineMessage>}
   {editor && usage === 'text' && (network || networkError || tested) && <AsyncStatusFeedback status={network ? 'pending' : networkError ? 'error' : 'success'} message={network ? '正在测试连接…' : networkError ?? (port.mode === 'native' ? '连接成功' : '模拟连接成功')}/>}
   {editor && usage === 'text' && <p className={s.meta}>{port.mode === 'native' ? '测试会发送一条短请求，可能产生少量费用。' : '浏览器测试为模拟结果。'}</p>}
   <div className={s.actions}>{editor ? <><Button type="submit" form={formId} variant="primary" loading={saving}>保存</Button>{usage === 'text' && <Button disabled={saving || network !== null} loading={network !== null} onClick={() => void check()}>测试连接</Button>}{network && <Button variant="quiet" onClick={() => {cancel(); setNetworkError('请求已取消。');}}>取消请求</Button>}{records.length > 0 && <Button disabled={saving} onClick={() => guard(() => setEditor(null))}>取消编辑</Button>}</> : usage === 'speech' && current && onSpeech && <Button variant="primary" disabled={saving || current.credentialStatus !== 'present'} onClick={() => guard(onSpeech)}>去首页试说</Button>}</div>
   {port.mode === 'simulation' && <p className={s.simulation}>浏览器模拟 · 保存仅在当前页面有效</p>}
  </footer>
  <Dialog confirmation open={leaving} onOpenChange={open => {if(!open) continueEditing();}} title="有未保存的修改" description="要保留这次填写吗？"><div className={s.actions}><Button variant="primary" onClick={continueEditing}>继续填写</Button><Button onClick={discard}>放弃修改</Button></div></Dialog>
  <Dialog confirmation open={!!confirmation} onOpenChange={open => {if(!open && !saving) setConfirmation(null);}} title={'删除“'+(confirmation?.record.name ?? '')+'”？'} description="删除后无法撤销。">
   {confirmation?.expectation.replacements.map(r => {const target = view?.configs.find(c => c.id === r.configId); return <p className={s.replacement} key={r.usage}>{label(r.usage)}将{target ? <>改用 <strong>{target.name}</strong><br/>{target.modelId}<br/>{target.baseUrl}</> : '清空，请重新选择模型。'}</p>;})}
   <div className={s.actions}><Button disabled={saving} onClick={() => setConfirmation(null)}>取消</Button><Button variant="primary" loading={saving} onClick={() => void remove()}>确认删除</Button></div>
  </Dialog>
 </Tabs>;
}
