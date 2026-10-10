import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
export const repository='qinghui316/Sayweave';
export function releaseIdentity(version,tag,commit){
 if(!/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(version)||tag!==`v${version}`||!/^[a-f0-9]{40}$/.test(commit))throw new Error('Release identity requires exact version, tag and full commit');
 return {version,tag,commit,installer:`Sayweave_${version}_x64-setup.exe`};
}
export function verifyBuild(identity,root,versions){
 if(versions.some(version=>version!==identity.version))throw new Error('Package, Rust, Tauri and lock versions differ');
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 if(git(['rev-parse','HEAD'])!==identity.commit||git(['rev-parse',`${identity.tag}^{commit}`])!==identity.commit)throw new Error('Build HEAD/tag differs from expected commit');
 execFileSync('git',['merge-base','--is-ancestor',identity.commit,'origin/main'],{cwd:root,stdio:'pipe'});
 if(git(['status','--porcelain','--untracked-files=no']))throw new Error('Formal release requires a clean workspace');
}
export function validateRelease(identity,manifest,receipt,assets){
 const installer=identity.installer,signatureName=`${installer}.sig`;
 const expected=[installer,signatureName,'latest.json','release-receipt.json','SHA256SUMS.txt'];
 if(assets.size!==expected.length||expected.some(name=>!assets.has(name)))throw new Error('Release asset inventory differs');
 if(manifest.version!==identity.version||receipt.version!==identity.version||receipt.tag!==identity.tag||receipt.commit!==identity.commit||receipt.installer!==installer||receipt.schemaVersion!==1)throw new Error('Release receipt identity differs');
 const target=manifest.platforms?.['windows-x86_64'];
 const expectedUrl=`https://github.com/${repository}/releases/download/${identity.tag}/${installer}`;
 if(target?.url!==expectedUrl||typeof target.signature!=='string')throw new Error('Installer URL/signature identity differs');
 const bytes=assets.get(installer),hash=createHash('sha512').update(bytes).digest('base64');
 if(bytes.length===0||bytes.length>128*1024*1024||bytes.length!==receipt.installerSize||hash!==receipt.installerSha512||target.installerSize!==bytes.length||target.installerSha512!==hash)throw new Error('Installer hash/size differs');
 if(assets.get(signatureName).toString('utf8').trim()!==target.signature)throw new Error('Manifest signature differs');
 const lines=assets.get('SHA256SUMS.txt').toString('utf8').trim().split(/\r?\n/),names=new Set();
 if(lines.length!==4)throw new Error('Checksum inventory must contain four entries');
 for(const line of lines){
  const match=/^([a-f0-9]{64}) {2}([A-Za-z0-9_.-]+)$/.exec(line);
  if(!match||!expected.slice(0,4).includes(match[2])||names.has(match[2]))throw new Error('Invalid or duplicate checksum identity');
  names.add(match[2]);
  if(createHash('sha256').update(assets.get(match[2])).digest('hex')!==match[1])throw new Error('Checksum differs');
 }
 return target;
}
export function releaseNotes(value){
 if(typeof value.version!=='string'||!Array.isArray(value.changes)||value.changes.some(change=>typeof change!=='string'))throw new Error('Invalid release notes');
 return `Sayweave ${value.version}\n\n${value.changes.map(change=>`- ${change}`).join('\n')}\n${value.installation?`\n${value.installation}\n`:''}`;
}
