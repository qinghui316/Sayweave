// Release jobs reuse only successful main CI evidence for their exact tag SHA.
import {execFileSync} from 'node:child_process';
import {mkdirSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {readMechanismEvidence} from './windows-update-artifacts.mjs';

const commit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();
if(!/^[a-f0-9]{40}$/.test(commit)||process.env.SAYWEAVE_RELEASE_COMMIT!==commit)throw Error('Release evidence requires the exact release commit');
execFileSync('git',['merge-base','--is-ancestor',commit,'origin/main']);
const output=process.argv[2];if(!output||process.argv.length!==3)throw Error('Pass one empty output directory');
const directory=path.resolve(output);mkdirSync(directory);
const repo='qinghui316/Sayweave';
const runs=JSON.parse(execFileSync('gh',['run','list','--repo',repo,'--workflow','windows-ci.yml','--commit',commit,'--branch','main','--event','push','--status','success','--limit','20','--json','databaseId,headSha,headBranch,status,conclusion,event'],{encoding:'utf8',timeout:60000}));
const run=runs.find(item=>item.headSha===commit&&item.headBranch==='main'&&item.event==='push'&&item.status==='completed'&&item.conclusion==='success'&&Number.isSafeInteger(item.databaseId));
if(!run)throw Error('Successful exact-commit main CI and Windows mechanism acceptance are required');
execFileSync('gh',['run','download',String(run.databaseId),'--repo',repo,'--name','windows-update-evidence','--dir',directory],{stdio:'inherit',timeout:120000});
const evidence=readMechanismEvidence(directory,commit);
writeFileSync(path.join(directory,'ci-run.json'),JSON.stringify({...run,repository:repo,componentSha256:evidence.tools.manifest.artifacts.component.sha256},null,2)+'\n');
console.log(`Reused exact main CI ${run.databaseId}; component ${evidence.tools.manifest.artifacts.component.sha256}`);
