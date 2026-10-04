import type {ModelInput, ModelUsage} from '../../contracts/models';

export const speechPresets = [
 {label:'Qwen Audio 3.1 · 实时识别', modelId:'qwen-audio-3.1-asr-flash-streaming', protocol:'qwen-audio-streaming', baseUrl:'https://dashscope.aliyuncs.com'},
 {label:'Qwen Audio 3.0 · 实时识别', modelId:'qwen-audio-3.0-asr-flash-streaming', protocol:'qwen-audio-streaming', baseUrl:'https://dashscope.aliyuncs.com'},
 {label:'豆包 Seed-ASR 2.0 · 实时识别', modelId:'Doubao-Seed-ASR-2.0', protocol:'doubao-asr-streaming', baseUrl:'https://openspeech.bytedance.com'},
] as const;

export function modelName(modelId:string, usage:ModelUsage):string {
 const name = usage === 'speech' ? speechPresets.find(p => p.modelId === modelId)?.label : undefined;
 return [...(name ?? (modelId || '文字模型'))].slice(0,64).join('');
}

export function initialModel(usage:ModelUsage):ModelInput {
 if(usage === 'speech') {
  const {label:name, ...preset} = speechPresets[0];
  return {...preset, name, usages:['speech']};
 }
 return {name:'文字模型', protocol:'openai-chat', baseUrl:'https://api.openai.com/v1', modelId:'', usages:['text']};
}
