import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import path from 'node:path';

const controllers=new Set(['.github/workflows/windows-ci.yml','docs/storage.md','docs/testing.md','docs/release.md','scripts/check-distribution.mjs','scripts/storage-cargo.mjs','scripts/storage-policy.mjs','scripts/storage-safety.ps1','scripts/storage.mjs','scripts/test-desktop.ps1','scripts/test-dictionary-desktop.mjs','scripts/test-language-actor.mjs','scripts/test-language-dashboard-live.mjs','scripts/test-models-desktop.mjs','scripts/test-speech-desktop.mjs','scripts/test-windows-installed-startup.mjs','scripts/test-windows-update.mjs','scripts/windows-update-packages.mjs','scripts/windows-update-startup.mjs','scripts/windows-update-debug-policy.mjs','scripts/windows-update-metadata.ps1','scripts/windows-update-metadata.cs','scripts/frozen-product-scope.mjs','scripts/windows-product-boundaries.mjs','tests/storage-policy.test.ts','tests/frozen-product-scope.test.ts','tests/windows-product-boundaries.test.ts','tests/windows-update-startup.test.ts']);
const normalized=value=>JSON.stringify(value,(_key,item)=>item&&typeof item==='object'&&!Array.isArray(item)?Object.fromEntries(Object.entries(item).sort(([a],[b])=>a.localeCompare(b))):item);
function withoutDevProfiles(text){
 const lines=text.replace(/^\uFEFF/,'').split(/\r?\n/),remaining=[];
 for(let i=0;i<lines.length;i++){
  const line=lines[i].trim();
  if(/^\[profile\.(dev|test)\]$/.test(line)){
   const body=[];while(i+1<lines.length&&!lines[i+1].trim().startsWith('[')){const value=lines[++i].trim();if(value&&!value.startsWith('#'))body.push(value.replace(/\s/g,''));}
   if(body.sort().join(';')!=='debug=1;incremental=true')throw Error('Unsupported development profile delta');
  }else if(line&&!line.startsWith('#'))remaining.push(line);
 }
 return remaining.join('\n');
}
export function assertFrozenFile(file,before,after){
 if(controllers.has(file))return 'acceptance-controller';
 if(['src-tauri/Cargo.toml','src-tauri/text-service/Cargo.toml'].includes(file)){
  if(withoutDevProfiles(before)!==withoutDevProfiles(after))throw Error('Frozen product Cargo dependency/release settings changed');return 'development-profile-only';
 }
 if(file==='package.json'){
  const old=JSON.parse(before),next=JSON.parse(after),allowed={'storage:plan':'node scripts/storage.mjs plan','storage:apply':'node scripts/storage.mjs apply','test:native':'node scripts/storage-cargo.mjs test --manifest-path src-tauri/Cargo.toml'};
  for(const [name,value] of Object.entries(allowed)){if(next.scripts[name]!==undefined&&next.scripts[name]!==old.scripts[name]&&next.scripts[name]!==value)throw Error('Unexpected verification command');delete old.scripts[name];delete next.scripts[name];}
  if(normalized(old)!==normalized(next))throw Error('Frozen package identity, dependencies or product commands changed');return 'storage-commands-only';
 }
 throw Error(`Frozen product source changed: ${file}`);
}
export function assertFrozenProduct(root,product,controller){
 if(![product,controller].every(value=>/^[a-f0-9]{40}$/.test(value)))throw Error('Exact product/controller identities required');
 const git=args=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 if(git(['rev-parse','HEAD'])!==controller||git(['status','--porcelain','--untracked-files=no']))throw Error('Frozen controller tree is dirty or differs');
 git(['merge-base','--is-ancestor',product,controller]);
 const changed=git(['diff','--name-only','--no-renames',product,controller]).split('\n').filter(Boolean),checks=[];
 for(const file of changed){
  const before=controllers.has(file)?'':git(['show',`${product}:${file}`]);
  checks.push({file,scope:assertFrozenFile(file,before,readFileSync(path.join(root,file),'utf8'))});
 }
 return {productCommit:product,acceptanceCommit:controller,checks,productRebuildRequired:false};
}
