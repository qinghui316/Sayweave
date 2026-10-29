import {test,expect,vi} from 'vitest';
import {invoke} from '@tauri-apps/api/core';
import {createDictationPorts} from '../src/adapters/dictation-tauri';
import {createSpeechPorts} from '../src/adapters/speech-tauri';
import {initialSpeech} from '../src/contracts/speech';
import {initialDictation} from '../src/contracts/dictation';
import {BrowserTranslation} from '../src/adapters/translation-tauri';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn(),isTauri:()=>true}));
test('native task snapshots accept the complete translation catalog and still reject malformed facts',async()=>{
 const catalog=await new BrowserTranslation().read(),call=vi.mocked(invoke);
 const snapshot={...initialDictation(),taskId:'translation-task',presentationId:'translation-view',purpose:'translation' as const,presentation:'recording' as const,targetLanguage:'en-US',translationTargets:catalog.languages.map(l=>l.code),translationSelectable:true};
 const dictation=createDictationPorts().dictation!;
 call.mockResolvedValue(snapshot);
 await expect(dictation.snapshot()).resolves.toEqual(snapshot);
 expect(call).toHaveBeenLastCalledWith('dictation_snapshot');
 for(const invalid of [{...snapshot,translationTargets:[null]},{...snapshot,presentationId:null}]){
  call.mockResolvedValue(invalid);await expect(dictation.snapshot()).rejects.toMatchObject({code:'protocol'});
 }
});
test('native actions carry the displayed task identity and reject stale operations without retry',async()=>{
 const call=vi.mocked(invoke);call.mockResolvedValue(undefined);
 const dictation=createDictationPorts().dictation!;
 await dictation.control('displayed-A','cancel');
 expect(call).toHaveBeenLastCalledWith('dictation_control',{taskId:'displayed-A',action:'cancel'});
 await dictation.copy('displayed-A','presentation-A');
 expect(call).toHaveBeenLastCalledWith('dictation_copy',{taskId:'displayed-A',presentationId:'presentation-A'});
 await dictation.dismiss('displayed-A','presentation-A');
 expect(call).toHaveBeenLastCalledWith('dictation_dismiss',{taskId:'displayed-A',presentationId:'presentation-A'});
 const speech=createSpeechPorts().speech;call.mockResolvedValue(initialSpeech());
 await speech.retryCurrent('displayed-A');
 expect(call).toHaveBeenLastCalledWith('speech_retry_current',{taskId:'displayed-A'});
 call.mockClear();call.mockRejectedValue({code:'staleSession'});
 await expect(dictation.control('displayed-A','finish')).rejects.toMatchObject({code:'staleSession'});
 expect(call).toHaveBeenCalledTimes(1);
});
