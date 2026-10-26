// Build-host tools only. No SDK, compiler or shader compiler is shipped to users.
import {createHash} from 'node:crypto';
import {createReadStream,createWriteStream,existsSync,mkdirSync,readdirSync,writeFileSync,lstatSync} from 'node:fs';
import {pipeline} from 'node:stream/promises';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
export const toolSources={
 shader:{file:'shaderc-20261024.zip',url:'https://storage.googleapis.com/shaderc/artifacts/prod/graphics_shader_compiler/shaderc/windows-vs2022-amd64-release/continuous/50/20261024-103042/install.zip',sha256:'f5063d73d5a866fae12eca81a03af1f3ec92c4b958ea0c1d840c48ec7ff9dc98',max:240*1024**2},
 vulkan:{file:'vulkan-headers-1.4.350.0.tar.gz',url:'https://codeload.github.com/KhronosGroup/Vulkan-Headers/tar.gz/refs/tags/vulkan-sdk-1.4.350.0',sha256:'70270d10bf2c1e074a06ee37a50b75d332993d1b80a1d9526eeed2da6d82ed22',max:4*1024**2},
 spirv:{file:'spirv-headers-1.4.350.0.tar.gz',url:'https://codeload.github.com/KhronosGroup/SPIRV-Headers/tar.gz/refs/tags/vulkan-sdk-1.4.350.0',sha256:'9905d9341f20388adb852c77dd982f2c4d539fd68e6c1f1bcebf034715f2d1d5',max:2*1024**2},
 loader:{file:'vulkan-1.official.def',url:'https://raw.githubusercontent.com/KhronosGroup/Vulkan-Loader/vulkan-sdk-1.4.350.0/loader/vulkan-1.def',sha256:'9ba339b7f5ee2df28487698a6840ecf095fac58b415c2c89ad9f161e9d316378',max:32*1024},
};
const required=['bin/glslc.exe','include/vulkan/vulkan.h','include/spirv/unified1/spirv.h','Lib/vulkan-1.lib','share/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake'];
async function digest(file){const hash=createHash('sha256');for await(const bytes of createReadStream(file))hash.update(bytes);return hash.digest('hex');}
async function download(directory,source){
 const file=path.join(directory,source.file);
 if(existsSync(file)&&!lstatSync(file).isSymbolicLink()&&await digest(file)===source.sha256)return file;
 const response=await fetch(source.url,{signal:AbortSignal.timeout(180000)});
 if(!response.ok||!response.body||Number(response.headers.get('content-length'))>source.max)throw Error('Pinned local build tool download unavailable');
 const temporary=file+'.part';let count=0;
 async function* bounded(){for await(const bytes of response.body){count+=bytes.length;if(count>source.max)throw Error('Build tool exceeds bound');yield bytes;}}
 await pipeline(bounded(),createWriteStream(temporary,{flags:'w'}));
 if(await digest(temporary)!==source.sha256)throw Error('Pinned build tool digest differs');
 const {renameSync}=await import('node:fs');renameSync(temporary,file);return file;
}
function run(program,args){const r=spawnSync(program,args,{encoding:'utf8',windowsHide:true});if(r.error)throw r.error;if(r.status!==0)throw Error(`Local build tool setup failed: ${path.basename(program)} ${r.stderr?.slice(-2000)}`);return r.stdout.trim();}
function extractTar(archive,directory){
 mkdirSync(directory,{recursive:true});
 const entries=run('tar',['-tf',archive]).split(/\r?\n/u);
 if(entries.some(p=>p.startsWith('/')||p.includes('\\')||p.split('/').some(c=>c==='..')))throw Error('Build tool archive path differs');
 run('tar',['-xf',archive,'-C',directory,'--strip-components=1']);
}
export async function setupLocalAsrTools(project=root){
 if(process.platform!=='win32')throw Error('Windows x64 local runtime build tools required');
 const tools=path.join(project,'.local/tools'),sdk=process.env.VULKAN_SDK??path.join(tools,'vulkan-minimal');
 if(required.every(file=>existsSync(path.join(sdk,file))))return {sdk,reused:true};
 if(process.env.VULKAN_SDK)throw Error('Configured VULKAN_SDK is incomplete');
 mkdirSync(tools,{recursive:true});mkdirSync(path.join(sdk,'bin'),{recursive:true});mkdirSync(path.join(sdk,'Lib'),{recursive:true});
 const shader=await download(tools,toolSources.shader);
 // Extract the sole compiler executable rather than the 231 MB compiler SDK.
 const code="Add-Type -AssemblyName System.IO.Compression.FileSystem;$z=[IO.Compression.ZipFile]::OpenRead($env:SAYWEAVE_SHADER_ARCHIVE);try{$e=@($z.Entries|Where-Object {$_.FullName -match '(^|/)bin/glslc.exe$'});if($e.Count -ne 1){throw 'Ambiguous glslc'};[IO.Compression.ZipFileExtensions]::ExtractToFile($e[0],$env:SAYWEAVE_SHADER_OUTPUT,$true)}finally{$z.Dispose()}";
 const extraction=spawnSync('powershell',['-NoProfile','-EncodedCommand',Buffer.from(code,'utf16le').toString('base64')],{env:{...process.env,SAYWEAVE_SHADER_ARCHIVE:shader,SAYWEAVE_SHADER_OUTPUT:path.join(sdk,'bin/glslc.exe')},windowsHide:true,encoding:'utf8'});
 if(extraction.status!==0)throw Error('glslc extraction failed');
 const headers=path.join(tools,'vulkan-headers-source'),spirv=path.join(tools,'spirv-headers-source');
 extractTar(await download(tools,toolSources.vulkan),headers);extractTar(await download(tools,toolSources.spirv),spirv);
 const vswhere=path.join(process.env['ProgramFiles(x86)'],'Microsoft Visual Studio/Installer/vswhere.exe');
 const vs=run(vswhere,['-latest','-products','*','-property','installationPath']);
 const cmake=path.join(vs,'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe');
 for(const [source,build] of [[headers,'vulkan-headers-cmake'],[spirv,'spirv-headers-cmake']]){
  const out=path.join(tools,build);
  run(cmake,['-S',source,'-B',out,`-DCMAKE_INSTALL_PREFIX=${sdk}`,'-DCMAKE_EXPORT_NO_PACKAGE_REGISTRY=ON']);run(cmake,['--install',out,'--config','Release']);
 }
 const versions=readdirSync(path.join(vs,'VC/Tools/MSVC')).filter(name=>/^\d+(\.\d+)+$/u.test(name)).sort();
 const librarian=path.join(vs,'VC/Tools/MSVC',versions.at(-1),'bin/Hostx64/x64/lib.exe');
 run(librarian,['/nologo','/machine:x64',`/def:${await download(tools,toolSources.loader)}`,`/out:${path.join(sdk,'Lib/vulkan-1.lib')}`]);
 if(!required.every(file=>existsSync(path.join(sdk,file))))throw Error('Local build tools incomplete');
 writeFileSync(path.join(sdk,'sources.json'),JSON.stringify(toolSources,null,2)+'\n','utf8');
 return {sdk,reused:false};
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))console.log(JSON.stringify(await setupLocalAsrTools()));
