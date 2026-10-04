import {test,expect} from '@playwright/test';
test('overlay recovery actions fit the reserved canvas and remain reachable after failure',async({page})=>{
 await page.setViewportSize({width:384,height:284});
 await page.goto('/tests/fixtures/voice-recovery.html');
 const recovery=page.getByRole('button',{name:'重试恢复剪贴板'});
 await recovery.click();
 await expect(recovery).toBeEnabled();
 for(const button of [recovery,page.getByRole('button',{name:'复制',exact:true}),page.getByRole('button',{name:'重新开始录音'})]){
  const rect=await button.boundingBox();expect(rect).not.toBeNull();
  expect(rect!.y).toBeGreaterThanOrEqual(0);expect(rect!.y+rect!.height).toBeLessThanOrEqual(284);
  expect(rect!.x).toBeGreaterThanOrEqual(0);expect(rect!.x+rect!.width).toBeLessThanOrEqual(384);
 }
 await expect(page.getByRole('status').first()).toContainText('剪贴板恢复未完成');
});
