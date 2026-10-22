import {render,screen} from '@testing-library/react';
import {it,expect} from 'vitest';
import {dictationFeedback} from '../src/app/dictation-feedback';
import {initialDictation,dictationPhase} from '../src/contracts/dictation';
import {initialSpeech} from '../src/contracts/speech';
import {VoiceSurface} from '../src/features/voice';
import {Home} from '../src/features/home';

it.each(['recording','completed','failed'] as const)('current external preparation ignores stale Runtime phase %s',phase=>{
 const snapshot={...initialDictation(),source:'external' as const,taskId:'new',presentationId:'new-view',stage:'recognizing' as const,presentation:'preparing' as const};
 expect(dictationPhase(snapshot,phase,{taskId:'old',text:'上一次口述'})).toBe('connecting');
});

it('clipboard failure keeps the result and original input facts visible',()=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'view',presentation:'result' as const,error:'copyFailed',text:'完整口述',delivery:{state:'unconfirmed' as const,submission:'uncertain' as const,verification:'unavailable' as const,stage:'composition',submittedEvents:0,error:'compositionPartial',restoreError:false,elapsedMs:1}};
 const message=dictationFeedback(value,initialSpeech());
 expect(message).toBe('复制失败，请重试。');
 render(<VoiceSurface presentation="result" phase="completed" text={value.text} delivery={value.delivery} error={message} copy={async()=>{}} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.getByRole('alert')).toHaveTextContent('复制失败，请重试。');
 expect(screen.getByText('完整口述')).toBeVisible();
 expect(screen.getByRole('button',{name:'复制'})).toBeEnabled();
 expect(value.delivery.submission).toBe('uncertain');
});

it('quiet native terminal cannot show an older source error during its pending hide',()=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'pending-hide',stage:'result' as const,error:'emptyText'};
 const speech={...initialSpeech(),taskId:'old',source:'history' as const,error:'network' as const};
 expect(dictationFeedback(value,speech)).toBeUndefined();
 expect(dictationPhase(value,'failed',speech)).toBe('idle');
 render(<VoiceSurface presentation="idle" phase="failed" error="旧任务错误" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();
 expect(screen.queryByRole('button')).not.toBeInTheDocument();
});
it('safety facts remain in the task without a blocking external bubble',()=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'view',presentation:'error' as const,error:'emptyText',delivery:{state:'unconfirmed' as const,submission:'uncertain' as const,verification:'unavailable' as const,stage:'composition',submittedEvents:0,error:'compositionCleanup',restoreError:true,elapsedMs:1}};
 const message=dictationFeedback(value,initialSpeech());
 expect(message).toBeUndefined();
 render(<VoiceSurface presentation="error" phase="failed" error={message} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();
 expect(screen.queryByText('识别未完成，请重试。')).not.toBeInTheDocument();
});
it('missing presentation detail cannot invent a recognition failure',()=>{
 render(<VoiceSurface presentation="error" phase="failed" start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
it.each(['compositionDisconnected','compositionRejected'])('an empty recognition result does not mask the real input error %s',code=>{
 const value={...initialDictation(),source:'external' as const,taskId:'current',presentationId:'view',presentation:'error' as const,error:'emptyText',delivery:{state:'unconfirmed' as const,submission:'notSubmitted' as const,verification:'unavailable' as const,stage:'composition',submittedEvents:0,error:code,restoreError:false,elapsedMs:1}};
 expect(dictationFeedback(value,initialSpeech())).not.toBe('未识别到文字。');
 expect(dictationFeedback(value,initialSpeech())).toContain(code==='compositionDisconnected'?'输入组件连接中断':'实时输入已中断');
});
it('a fresh start failure remains visible while the previous trial terminal stays quiet',()=>{
 render(<Home quietTerminal phase="failed" speech={{...initialSpeech(),taskId:'old',error:'emptyText'}} failure="无法访问麦克风，请检查权限。" onShortcuts={()=>{}} onModels={()=>{}} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.getByRole('alert')).toHaveTextContent('无法访问麦克风');
 expect(screen.queryByText('未识别到文字。')).not.toBeInTheDocument();
});

it('archive failure does not add a trial warning or empty copy button',()=>{
 render(<Home phase="completed" speech={{...initialSpeech(),taskId:'trial',source:'trial',phase:'completed',saveError:true}} copy={async()=>{}} onShortcuts={()=>{}} onModels={()=>{}} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);
 expect(screen.queryByRole('alert')).not.toBeInTheDocument();expect(screen.queryByRole('button',{name:'复制'})).not.toBeInTheDocument();
});
