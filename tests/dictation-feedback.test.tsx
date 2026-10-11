import {render,screen} from '@testing-library/react';
import {it,expect} from 'vitest';
import {dictationFeedback} from '../src/app/dictation-feedback';
import {initialDictation,dictationPhase} from '../src/contracts/dictation';
import {initialSpeech} from '../src/contracts/speech';
import {VoiceSurface} from '../src/features/voice';

it('quiet native terminal cannot show an older source error during its pending hide',()=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'pending-hide',stage:'result' as const,error:'emptyText'};
 const speech={...initialSpeech(),taskId:'old',source:'history' as const,error:'network' as const};
 expect(dictationFeedback(value,speech)).toBeUndefined();
 expect(dictationPhase(value,'failed',speech)).toBe('idle');
 render(<VoiceSurface presentation="idle" phase="failed" error="旧任务错误" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();
 expect(screen.queryByRole('button')).not.toBeInTheDocument();
});
it('specific safety errors win over an empty recognition result and never use the generic fallback',()=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'view',presentation:'error' as const,error:'emptyText',delivery:{state:'unconfirmed' as const,submission:'uncertain' as const,verification:'unavailable' as const,stage:'composition',submittedEvents:0,error:'compositionCleanup',restoreError:true,elapsedMs:1}};
 const message=dictationFeedback(value,initialSpeech());
 expect(message).toContain('恢复未完成');
 render(<VoiceSurface presentation="error" phase="failed" error={message} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.getByRole('alert')).toHaveTextContent('输入恢复未完成');
 expect(screen.queryByText('识别未完成，请重试。')).not.toBeInTheDocument();
});
it('missing presentation detail cannot invent a recognition failure',()=>{
 render(<VoiceSurface presentation="error" phase="failed" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
