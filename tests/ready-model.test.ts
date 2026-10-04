import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {BrowserModels} from '../src/adapters/models-browser';
import {BrowserSpeech} from '../src/adapters/speech-browser';
import {ModelError,type ModelConfigPort} from '../src/contracts/models';
import {readReadyModel} from '../src/app/ready-model';
beforeEach(()=>vi.useFakeTimers());afterEach(()=>vi.useRealTimers());

it('recovers a transient status/overview lock without losing the selected model and device',async()=>{
 const models=new BrowserModels(),speech=new BrowserSpeech();
 await models.save({input:{name:'语音',protocol:'qwen-audio-streaming',baseUrl:'https://example.com',modelId:'speech-model',usages:['speech']},id:null,revision:null,key:{action:'replace',value:'fixture-key'},expectation:null});
 const status=models.status.bind(models),overview=models.overview.bind(models);
 vi.spyOn(models,'status').mockRejectedValueOnce(new ModelError('busy')).mockImplementation(status);
 vi.spyOn(models,'overview').mockRejectedValueOnce(new ModelError('busy')).mockImplementation(overview);
 const result=readReadyModel(models,speech,()=>true);await vi.advanceTimersByTimeAsync(300);
 expect(await result).toEqual({model:'speech-model',device:'模拟麦克风（不采集声音）'});expect(models.status).toHaveBeenCalledTimes(3);
});
it('invalidated startup reads do not query metadata again or return stale results',async()=>{
 const models=new BrowserModels(),speech=new BrowserSpeech();let current=true;
 vi.spyOn(models,'status').mockRejectedValueOnce(new ModelError('busy'));const overview=vi.spyOn(models,'overview');
 const result=readReadyModel(models,speech,()=>current);await vi.advanceTimersByTimeAsync(0);current=false;await vi.advanceTimersByTimeAsync(100);
 expect(await result).toBeNull();expect(overview).not.toHaveBeenCalled();expect(models.status).toHaveBeenCalledTimes(1);
});
it('does not retry permanent failures or perform any writes',async()=>{
 const models=new BrowserModels(),speech=new BrowserSpeech();vi.spyOn(models,'status').mockRejectedValue(new ModelError('forbidden'));const save=vi.spyOn(models,'save');
 await expect(readReadyModel(models,speech,()=>true)).rejects.toMatchObject({code:'forbidden'});expect(models.status).toHaveBeenCalledTimes(1);expect(save).not.toHaveBeenCalled();
});
it('bounds preparing reads rather than scheduling forever',async()=>{
 const models:ModelConfigPort=new BrowserModels();const speech=new BrowserSpeech();vi.spyOn(models,'status').mockResolvedValue({state:'preparing',schemaVersion:5,cleanupPending:false});
 const settled=readReadyModel(models,speech,()=>true).catch(error=>error);await vi.advanceTimersByTimeAsync(1000);
 expect(await settled).toMatchObject({code:'preparing'});expect(models.status).toHaveBeenCalledTimes(10);
});
