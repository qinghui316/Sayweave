import {ModelError,modelError,modelExecutable,type ModelConfigPort} from '../contracts/models';
import type {SpeechPort} from '../contracts/speech';

// Read-only startup metadata; a newer focus/settings action invalidates this read.
export async function readReadyModel(models:ModelConfigPort,speech:SpeechPort,current:()=>boolean){
 for(let attempt=0;attempt<10;attempt++){
  if(!current())return null;
  try{
   const status=await models.status();if(!current())return null;
   if(status.state!=='ready')throw new ModelError(status.state);
   const [overview,prefs,devices]=await Promise.all([models.overview(),speech.preferences(),speech.devices()]);if(!current())return null;
   const model=overview.configs.find(c=>c.id===overview.defaults.find(d=>d.usage==='speech')?.configId);
   const device=prefs.deviceId?devices.find(d=>d.id===prefs.deviceId):devices.find(d=>d.isDefault);
   return {model:model&&modelExecutable(model,'speech')?model.modelId:'',device:device?.name??(prefs.deviceId?'麦克风不可用':'系统默认麦克风')};
  }catch(e){if(!current())return null;const code=modelError(e).code;if(attempt===9||!['busy','preparing'].includes(code))throw e;await new Promise(resolve=>setTimeout(resolve,100));}
 }
 return null;
}
