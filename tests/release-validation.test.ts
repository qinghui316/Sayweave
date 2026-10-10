import {createHash} from 'node:crypto';
import {describe,expect,it} from 'vitest';
// The public release tools are JavaScript entry points with separately exercised inputs.
// @ts-expect-error JavaScript release validator has no declaration file.
import {releaseIdentity,validateRelease,releaseNotes} from '../scripts/release-validation.mjs';
function fixture(){
 const identity=releaseIdentity('0.1.7','v0.1.7','a'.repeat(40)),bytes=Buffer.from('synthetic installer'),signature='synthetic signature';
 const sha512=createHash('sha512').update(bytes).digest('base64');
 const target={signature,url:'https://github.com/qinghui316/Sayweave/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe',installerSize:bytes.length,installerSha512:sha512};
 const manifest={version:'0.1.7',platforms:{'windows-x86_64':target}};
 const receipt={schemaVersion:1,...identity,installerSize:bytes.length,installerSha512:sha512};
 const assets=new Map<string,Buffer>([[identity.installer,bytes],[`${identity.installer}.sig`,Buffer.from(signature)],['latest.json',Buffer.from(JSON.stringify(manifest))],['release-receipt.json',Buffer.from(JSON.stringify(receipt))]]);
 const sums=()=>[...assets].filter(([name])=>name!=='SHA256SUMS.txt').map(([name,bytes])=>`${createHash('sha256').update(bytes).digest('hex')}  ${name}`).join('\n');
 assets.set('SHA256SUMS.txt',Buffer.from(sums()));return{identity,manifest,receipt,assets,sums};
}
describe('public release identity and inventory',()=>{
 it('accepts an exact inventory and generates readable notes',()=>{const f=fixture();expect(validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toBe(f.manifest.platforms['windows-x86_64']);expect(releaseNotes({version:'0.1.7',changes:['修复更新']})).toContain('- 修复更新');});
 it.each(['unknown','b'.repeat(40),'a'.repeat(39)])('rejects a false receipt commit %s',commit=>{const f=fixture();f.receipt.commit=commit;expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it.each(['https://evil.example/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe','https://github.com/qinghui316/Sayweave/releases/download/v0.1.7/wrong.exe'])('rejects an ambiguous URL %s',url=>{const f=fixture();f.manifest.platforms['windows-x86_64'].url=url;expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it.each(['missing','duplicate','traversal','extra'])('rejects %s checksum entries',kind=>{const f=fixture(),lines=f.sums().split('\n');if(kind==='missing')lines.pop();if(kind==='duplicate')lines[3]=lines[0]!;if(kind==='traversal')lines[0]=lines[0]!.replace(f.identity.installer,'../bad.exe');if(kind==='extra')lines.push(lines[0]!);f.assets.set('SHA256SUMS.txt',Buffer.from(lines.join('\n')));expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();});
 it('rejects extra assets and a version without an exact commit',()=>{const f=fixture();f.assets.set('extra.exe',Buffer.from('extra'));expect(()=>validateRelease(f.identity,f.manifest,f.receipt,f.assets)).toThrow();expect(()=>releaseIdentity('0.1.7','v0.1.7','unknown')).toThrow();});
});
