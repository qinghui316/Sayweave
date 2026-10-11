import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {describe,expect,it} from 'vitest';

const template=readFileSync('src-tauri/windows/installer.nsi','utf8').replaceAll('\r\n','\n');
const section=(name:string)=>{
 const start=template.indexOf(`Section ${name}\n`),end=template.indexOf('SectionEnd',start);
 if(start<0||end<start)throw new Error(`Missing installer section: ${name}`);
 return template.slice(start,end);
};
describe('installer ownership and ordering',()=>{
 it.skipIf(process.platform!=='win32')('binds the pinned offset to the actual installed CLI template bytes',()=>{
  const pin=JSON.parse(readFileSync('src-tauri/windows/installer-source.json','utf8')) as {templateOffset:number;templateLength:number;templateSha256:string;bindingSha256:string};
  const binary=readFileSync('node_modules/@tauri-apps/cli-win32-x64-msvc/cli.win32-x64-msvc.node');
  const digest=(value:Buffer)=>createHash('sha256').update(value).digest('hex');
  expect(digest(binary)).toBe(pin.bindingSha256);
  expect(digest(binary.subarray(pin.templateOffset,pin.templateOffset+pin.templateLength))).toBe(pin.templateSha256);
  expect(binary[pin.templateOffset+pin.templateLength]).toBe(0);
 });
 it('backs up before any application copy and commits after metadata and shortcuts',()=>{
  const install=section('Install');
  expect(install.indexOf('Call SayweaveBegin')).toBeLessThan(install.indexOf('File "${MAINBINARYSRCPATH}"'));
  expect(install.indexOf('--sayweave-installer=commit')).toBeGreaterThan(install.indexOf('WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion"'));
  expect(install).not.toContain('CheckIfAppIsRunning');
  expect(template).not.toContain("ExecWait '$R1'");
 });
 it('routes true uninstall through the locked controller with no second destructive pass',()=>{
  const uninstall=section('Uninstall');
  expect(uninstall).toContain('--sayweave-installer=uninstall');
  expect(uninstall).not.toMatch(/\bDelete\b|\bDeleteRegKey\b|CheckIfAppIsRunning|RmDir\s+\/r/);
  expect(template).toContain('SayweaveRejectMsi');
  expect(template).toContain('nsis_tauri_utils::SemverCompare "${VERSION}" $R0');
  expect(template).toContain('fixed root start-menu shortcut only');
 });
});
