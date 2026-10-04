import {describe,it,expect} from 'vitest';
import fixture from './model-capabilities.json';
import {modelProtocols,modelExecutable,isModelProtocol,validateModelInput} from '../../src/contracts/models';
describe('shared model protocol capabilities',()=>{
 it('matches the same fixed samples consumed by Rust',()=>{
  expect(modelProtocols).toEqual(fixture);
  for(const [protocol,capability] of Object.entries(fixture)) {
   expect(isModelProtocol(protocol)).toBe(true);
   for(const usage of ['speech','text'] as const)expect(modelExecutable({name:'fixture',baseUrl:'https://example.invalid',modelId:'fixture',protocol:protocol as keyof typeof modelProtocols,usages:[usage]},usage)).toBe(capability.usages.includes(usage));
  }
  expect(isModelProtocol('unknown')).toBe(false);expect(isModelProtocol('constructor')).toBe(false);
 });
 it('validates the executable speech model mapping and scope',()=>{
  const input={name:'豆包',protocol:'doubao-asr-streaming' as const,baseUrl:'https://openspeech.bytedance.com',modelId:'Doubao-Seed-ASR-2.0',usages:['speech' as const]};
  expect(validateModelInput(input)).toEqual(input);
  expect(()=>validateModelInput({...input,usages:['text']})).toThrow();
  expect(()=>validateModelInput({...input,modelId:'bigmodel'})).toThrow();
  expect(()=>validateModelInput({...input,baseUrl:input.baseUrl+'/v1'})).toThrow();
 });
});
