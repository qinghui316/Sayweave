// Release jobs reuse successful main mechanism evidence for their exact tag SHA.
import {execFileSync} from 'node:child_process';
import {mkdirSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {readMechanismEvidence} from './windows-update-artifacts.mjs';

export function findMechanismRun(runs,commit,inventory){
 if(!Array.isArray(runs)||runs.length>20)throw Error('Invalid main run inventory');
 for(const candidate of runs){
  if(candidate.headSha!==commit||candidate.headBranch!=='main'||!['push','workflow_dispatch'].includes(candidate.event)||candidate.status!=='completed'||candidate.conclusion!=='success'||!Number.isSafeInteger(candidate.databaseId)||candidate.databaseId<=0)continue;
  const assets=inventory(candidate.databaseId);
  if(!Array.isArray(assets.artifacts)||!Number.isSafeInteger(assets.total_count)||assets.total_count<0||assets.total_count>100||assets.artifacts.length!==assets.total_count)throw Error('Invalid mechanism artifact inventory');
  const evidence=assets.artifacts.filter(item=>item.name==='windows-update-evidence');
  if(evidence.length>1)throw Error('Ambiguous mechanism evidence');
  if(evidence.length===1&&evidence[0].expired===false)return candidate;
 }
 throw Error('Successful exact-commit main CI and Windows mechanism acceptance are required');
}

function main(){
const commit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();
if(!/^[a-f0-9]{40}$/.test(commit)||process.env.SAYWEAVE_RELEASE_COMMIT!==commit)throw Error('Release evidence requires the exact release commit');
execFileSync('git',['merge-base','--is-ancestor',commit,'origin/main']);
const output=process.argv[2];if(!output||process.argv.length!==3)throw Error('Pass one empty output directory');
const directory=path.resolve(output);mkdirSync(path.dirname(directory),{recursive:true});mkdirSync(directory);
const repo='qinghui316/Sayweave';
const runs=JSON.parse(execFileSync('gh',['run','list','--repo',repo,'--workflow','windows-ci.yml','--commit',commit,'--branch','main','--status','success','--limit','20','--json','databaseId,headSha,headBranch,status,conclusion,event'],{encoding:'utf8',timeout:60000}));
const run=findMechanismRun(runs,commit,id=>JSON.parse(execFileSync('gh',['api',`repos/${repo}/actions/runs/${id}/artifacts?per_page=100`],{encoding:'utf8',timeout:60000})));
execFileSync('gh',['run','download',String(run.databaseId),'--repo',repo,'--name','windows-update-evidence','--dir',directory],{stdio:'inherit',timeout:120000});
const evidence=readMechanismEvidence(directory,commit);
writeFileSync(path.join(directory,'ci-run.json'),JSON.stringify({...run,repository:repo,componentSha256:evidence.tools.manifest.artifacts.component.sha256},null,2)+'\n');
console.log(`Reused exact main CI ${run.databaseId}; component ${evidence.tools.manifest.artifacts.component.sha256}`);
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))main();
