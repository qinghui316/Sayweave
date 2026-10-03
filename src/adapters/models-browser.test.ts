import {describe,it,expect} from 'vitest';
import {BrowserModels} from './models-browser';
import {modelExpectation,parseModelOverview,validateModelInput,type ModelInput,type ModelSave} from '../contracts/models';
const input=(name='one'):ModelInput=>({name,protocol:'openai-chat',baseUrl:'https://example.invalid/v1',modelId:'text-1',usages:['speech','text']});
const create=(name='one'):ModelSave=>({input:input(name),key:{action:'replace',value:'fixture-key'},id:null,revision:null,expectation:null});
describe('model configuration memory contract',()=>{
 it('fills vacant defaults only, guards replacement preview and revisions',async()=>{
  const p=new BrowserModels(),a=await p.save(create()),id=a.configs[0]!.id;
  const b=await p.save(create('two')),next=b.configs.find(c=>c.id!==id)!;
  expect(b.defaults.every(d=>d.configId===id)).toBe(true);
  const pending={id,revision:1,expectation:modelExpectation(b,id,[])};
  await p.setDefault({usage:'text',configId:next.id,revision:b.defaults[1]!.revision});
  await expect(p.remove(pending)).rejects.toMatchObject({code:'conflict'});
  const c=await p.overview(),deleted=await p.remove({id,revision:1,expectation:modelExpectation(c,id,[])});
  expect(deleted.defaults.every(d=>d.configId===next.id)).toBe(true);
  const cleared=await p.remove({id:next.id,revision:1,expectation:modelExpectation(deleted,next.id,[])});
  expect(cleared.defaults.every(d=>d.configId===null)).toBe(true);
 });
 it('rejects retaining keys across origins and never returns a stored key',async()=>{
  const p=new BrowserModels(),a=await p.save(create()),c=a.configs[0]!;
  const d:ModelSave={input:{...input(),baseUrl:'https://other.invalid/v1'},id:c.id,revision:1,key:{action:'keep'},expectation:modelExpectation(a,c.id,['speech','text'])};
  await expect(p.save(d)).rejects.toMatchObject({code:'originChanged'});
  d.input.baseUrl='https://example.invalid/v2';const result=await p.save(d);
  expect(result.configs[0]!.revision).toBe(2);expect(JSON.stringify(result)).not.toContain('fixture-key');
  await expect(p.save(d)).rejects.toMatchObject({code:'conflict'});
 });
 it('cancels a simulated in-flight test and marks no config as tested',async()=>{
  const p=new BrowserModels(),id=crypto.randomUUID();
  const operation=p.testTextModel(create(),id);await p.cancelRequest(id);
  await expect(operation).rejects.toMatchObject({code:'cancelled'});expect((await p.overview()).configs).toHaveLength(0);
 });
 it('validates canonical URL, unicode and rejects malformed native defaults',()=>{
  expect(validateModelInput({...input(),name:' 名称 ',baseUrl:'https://example.invalid/v1///'}).baseUrl).toBe('https://example.invalid/v1');
  for(const baseUrl of ['http://localhost/v1','https://a:b@example.com','https://example.com/?q=x','https://example.com/#x'])expect(()=>validateModelInput({...input(),baseUrl})).toThrow();
  expect(()=>validateModelInput({...input(),name:'a\u2028b'})).toThrow();
  expect(()=>parseModelOverview({configs:[],defaults:[],cleanupPending:false})).toThrow();
 });
});

