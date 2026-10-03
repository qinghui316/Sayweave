import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
const bad=[], decoder=new TextDecoder('utf-8',{fatal:true});
const garble=/\u9505|\u951b|\u9286|\u9983|\u8133|\u7459|\u8930|\u95c6|\u9365|\u9359|\u9366|\u93c8\ue1c7|\ufffd/;
function scan(dir) {
  for(const item of readdirSync(dir,{withFileTypes:true})) {
    const file=path.join(dir,item.name);
    if(item.isDirectory()) { if(!['icons','target','gen','assets'].includes(item.name))scan(file); }
    else if(/\.(tsx?|rs|css|mjs|ps1|md|json|toml)$/.test(item.name)) {
      try { if(garble.test(decoder.decode(readFileSync(file))))bad.push(file); } catch { bad.push(file); }
    }
  }
}
['src','src-tauri','scripts','tests','docs'].forEach(dir=>scan(dir));
if(bad.length) { console.error(`UTF-8/garble check failed: ${bad.join(', ')}`);process.exit(1); }
console.log('UTF-8 and garble check passed');
