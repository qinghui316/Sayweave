import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {readFile,stat} from 'node:fs/promises';
import {join,resolve} from 'node:path';
const root=process.cwd();const dir=resolve(root,process.env.SAYWEAVE_RELEASE_DIR??'release/sayweave');
const pkg=JSON.parse(await readFile(resolve(root,'package.json'),'utf8'));const manifest=JSON.parse(await readFile(join(dir,'latest.json'),'utf8'));const receipt=JSON.parse(await readFile(join(dir,'release-receipt.json'),'utf8'));
if(manifest.version!==pkg.version||receipt.version!==pkg.version)throw new Error('Release version mismatch');
const target=manifest.platforms?.['windows-x86_64'];if(!target?.url||!target.signature)throw new Error('Windows update manifest incomplete');
const installer=receipt.installer;const bytes=await readFile(join(dir,installer));const sha512=createHash('sha512').update(bytes).digest('base64');
if(sha512!==receipt.installerSha512||bytes.length!==receipt.installerSize)throw new Error('Installer hash/size mismatch');
const signaturePath=join(dir,`${installer}.sig`);if((await stat(signaturePath)).size<20)throw new Error('Installer signature is empty');const signature= (await readFile(signaturePath,'utf8')).trim();if(signature!==target.signature.trim())throw new Error('Manifest signature does not match installer signature');
const config=JSON.parse(await readFile(resolve(root,'src-tauri/tauri.conf.json'),'utf8'));const publicKey=config.plugins?.updater?.pubkey;if(typeof publicKey!=='string'||!publicKey)throw new Error('Updater public key is missing');
const verify=spawnSync('cargo',['run','--quiet','--manifest-path','src-tauri/Cargo.toml','--bin','verify-update-signature','--',publicKey,join(dir,installer),signaturePath],{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe']});
if(verify.status!==0)throw new Error(`Updater signature verification failed: ${verify.stderr||verify.stdout}`);
if(!target.url.includes(`/releases/download/${receipt.tag}/`))throw new Error('Installer URL is not bound to release tag');
const sums=(await readFile(join(dir,'SHA256SUMS.txt'),'utf8')).trim().split(/\r?\n/);for(const line of sums){const match=/^([a-f0-9]{64}) {2}(.+)$/.exec(line);if(!match)throw new Error('Invalid checksum line');const actual=createHash('sha256').update(await readFile(join(dir,match[2]))).digest('hex');if(actual!==match[1])throw new Error(`Checksum mismatch: ${match[2]}`);}
console.log(`Verified Sayweave release ${manifest.version} (${receipt.tag})`);
