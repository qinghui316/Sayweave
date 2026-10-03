import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
const findings=[];
if(!process.version.startsWith('v24.'))findings.push('Use project Node24 through scripts/environment.ps1');
for(const file of ['node_modules/.bin/tauri.cmd','package-lock.json','rust-toolchain.toml','src-tauri/Cargo.lock'])if(!existsSync(file))findings.push(`Missing ${file}`);
for(const bin of ['rustc','cargo']) { const r=spawnSync(bin,['--version'],{encoding:'utf8'});if(r.status!==0)findings.push(`${bin}: missing project toolchain environment`);else console.log(r.stdout.trim()); }
console.log(`Node ${process.version}; app ${JSON.parse(readFileSync('package.json','utf8')).version}`);
if(findings.length) { console.error(findings.join('\n'));process.exit(1); }
console.log('Project environment passed; OS C++/WebView2 verified by native compilation/run separately.');
