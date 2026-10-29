import type {DictationSnapshot} from '../contracts/dictation';
import type {SpeechSnapshot} from '../contracts/speech';
import {componentMessage} from '../contracts/text-service';
import {speechMessages} from '../locales/speech';

/** Only the native owner of this presentation can supply its terminal error. */
export function dictationFeedback(value:DictationSnapshot,speech:SpeechSnapshot):string|undefined {
 if(value.purpose==='translation'&&value.source==='external'&&value.presentation==='result') {
  if(!value.translationComplete)return value.error==='emptyText'?'未识别到文字。':'翻译未完成，已保留识别原文。';
  if(value.delivery?.submission==='uncertain')return '填入结果无法确认，请检查原输入框，避免重复粘贴。';
  if(value.delivery?.error)return '输入位置已变化或无法核验，请复制译文。';
 }
 if(value.source==='external'&&value.presentation==='result')return value.error==='copyFailed'?speechMessages.copyFailed:undefined;
 if(value.source!=='external'||value.presentation!=='error')return undefined;
 const delivery=value.delivery;
 if(value.error==='saveFailed'||delivery?.restoreError||delivery?.submission==='uncertain'||delivery?.error&&['compositionCleanup','compositionRollback','compositionHandoffFailed','targetReadonlyContext'].includes(delivery.error)||value.error==='targetReadonlyContext')return undefined;
 const cleanup=delivery?.error&&['compositionCleanup','compositionRollback','compositionHandoffFailed'].includes(delivery.error)?delivery.error:null;
 const normalSpeech=value.error!==null&&['emptyText','cancelled','staleSession'].includes(value.error);
 const inputFailure=normalSpeech&&delivery?.error&&!['inputUnconfirmed','cancelled','emptyText','staleSession','editing','targetChanged','targetLost','targetClosed'].includes(delivery.error)?delivery.error:null;
 const code=cleanup??inputFailure??value.error??delivery?.error??(speech.taskId===value.taskId&&speech.source==='external'?speech.error:null);
 if(!code)return undefined;
 if(code in speechMessages)return speechMessages[code as keyof typeof speechMessages];
 return componentMessage(code);
}
