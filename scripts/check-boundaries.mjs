import ts from 'typescript';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
const root=path.resolve('src');
function walk(dir) { return readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name)):/\.(ts|tsx)$/.test(e.name)?[path.join(dir,e.name)]:[]); }
const files=walk(root), errors=[], graph=new Map();
const relative=file=>path.relative(root,file).replaceAll('\\','/');
for (const file of files) {
  const from=relative(file), imports=[];
  const ast=ts.createSourceFile(file,readFileSync(file,'utf8'),ts.ScriptTarget.Latest,true);
  function visit(node) {
    let spec;
    if ((ts.isImportDeclaration(node)||ts.isExportDeclaration(node))&&node.moduleSpecifier&&ts.isStringLiteral(node.moduleSpecifier)) spec=node.moduleSpecifier.text;
    if (ts.isCallExpression(node)&&node.expression.kind===ts.SyntaxKind.ImportKeyword&&node.arguments[0]&&ts.isStringLiteral(node.arguments[0])) spec=node.arguments[0].text;
    if(spec) imports.push(spec); ts.forEachChild(node,visit);
  } visit(ast);
  const edges=[];
  for (const spec of imports) {
    if(spec.startsWith('@tauri-apps/')&&!from.startsWith('adapters/')) errors.push(`${from}: native import outside adapters: ${spec}`);
    if(!spec.startsWith('.')) continue;
    const base=path.resolve(path.dirname(file),spec);
    const target=[base,base+'.ts',base+'.tsx',path.join(base,'index.ts'),path.join(base,'index.tsx')].find(p=>files.includes(p));
    if(!target) continue;
    const to=relative(target); edges.push(target);
    if(from.startsWith('ui/')&&/^(features|app|adapters|dev)\//.test(to)) errors.push(`${from} -> ${to}: UI boundary`);
    if(from.startsWith('features/')) {
      const feature=from.split('/')[1];
      if(/^(app|adapters|dev)\//.test(to)) errors.push(`${from} -> ${to}: feature boundary`);
      if(to.startsWith('features/')&&to.split('/')[1]!==feature) errors.push(`${from} -> ${to}: cross-feature import`);
    }
    if(!from.startsWith('app/')&&to.startsWith('features/')&&!from.startsWith('features/')) errors.push(`${from} -> ${to}: application owns feature composition`);
    if(to.startsWith('features/')&&!from.startsWith('features/')&&!/features\/[^/]+\/index\.tsx?$/.test(to)) errors.push(`${from} -> ${to}: private feature import`);
  } graph.set(file,edges);
}
const visiting=new Set(), done=new Set();
function inspect(file,trail=[]) { if(visiting.has(file)) { errors.push(`cycle: ${[...trail,file].map(relative).join(' -> ')}`);return; } if(done.has(file))return; visiting.add(file); for(const e of graph.get(file)||[]) inspect(e,[...trail,file]); visiting.delete(file);done.add(file); }
files.forEach(f=>inspect(f));
if(errors.length) { console.error(errors.join('\n'));process.exit(1); }
console.log(`Module boundaries and cycles: ${files.length} sources passed`);
