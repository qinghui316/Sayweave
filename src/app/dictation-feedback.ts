import type {DictationSnapshot} from '../contracts/dictation';
import type {SpeechSnapshot} from '../contracts/speech';
import {componentMessage} from '../contracts/text-service';
import {speechMessages} from '../locales/speech';

/** Only the native owner of this presentation can supply its terminal error. */
export function dictationFeedback(value:DictationSnapshot,speech:SpeechSnapshot):string|undefined {
 if(value.source!=='external'||value.presentation!=='error')return undefined;
 const delivery=value.delivery;
 if(value.error==='saveFailed')return speechMessages.saveFailed;
 if(delivery?.restoreError)return '输入恢复未完成，请检查目标输入框并保留现有文字。';
 if(delivery?.submission==='uncertain')return '无法确认文字是否已填入，请检查输入框，避免重复输入。';
 const cleanup=delivery?.error&&['compositionCleanup','compositionRollback','compositionHandoffFailed'].includes(delivery.error)?delivery.error:null;
 const normalSpeech=value.error!==null&&['emptyText','cancelled','staleSession'].includes(value.error);
 const inputFailure=normalSpeech&&delivery?.error&&!['inputUnconfirmed','cancelled','emptyText','staleSession','editing','targetChanged','targetLost','targetClosed'].includes(delivery.error)?delivery.error:null;
 const code=cleanup??inputFailure??value.error??delivery?.error??(speech.taskId===value.taskId&&speech.source==='external'?speech.error:null);
 if(!code)return undefined;
 if(code in speechMessages)return speechMessages[code as keyof typeof speechMessages];
 if(code==='compositionCleanup'||code==='compositionRollback'||code==='compositionHandoffFailed')return '输入草稿未能安全结算，请检查目标输入框并保留现有文字。';
 if(code==='targetReadonlyContext')return '此输入框当前不能编辑，请重新聚焦可编辑的输入位置。';
 return componentMessage(code);
}
