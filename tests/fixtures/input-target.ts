import {Schema} from 'prosemirror-model';
import {EditorState, Plugin, PluginKey, TextSelection} from 'prosemirror-state';
import {Decoration, DecorationSet, EditorView} from 'prosemirror-view';
import {createElement, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {flushSync} from 'react-dom';

function semanticSelection(editor:EditorView){const offset=(position:number)=>{let blocks=0;editor.state.doc.forEach((_node,start,index)=>{if(start<position)blocks=index;});return position-1-blocks;};return {start:offset(editor.state.selection.from),end:offset(editor.state.selection.to)};}

const schema=new Schema({nodes:{doc:{content:'paragraph+'},paragraph:{content:'text*',group:'block',parseDOM:[{tag:'p'}],toDOM:()=>['p',0]},text:{group:'inline'}}});
const host=document.createElement('div');host.id='prosemirror';document.body.append(host);
// The official view stylesheet requires preserved whitespace for native editing.
// This minimal fixture owns only paragraph/text nodes; other application UI is unchanged.
const style=document.createElement('style');style.textContent='.ProseMirror{position:relative;white-space:pre-wrap;white-space:break-spaces;word-wrap:break-word;font-variant-ligatures:none;}';document.head.append(style);
const view=new EditorView(host,{state:EditorState.create({schema})});
// Owned regression for a floating non-editable suggestion in an empty field.
// The suggestion disappears for IME key 229 or when committed text arrives.
const predictionKey=new PluginKey<boolean>('owned-floating-prediction');
function prediction(text:string){
 const reset=()=>requestAnimationFrame(()=>{if(!view.isDestroyed&&!view.composing)view.dispatch(view.state.tr.setMeta(predictionKey,false));});
 return new Plugin<boolean>({
  key:predictionKey,
  state:{init:()=>false,apply:(tr,hidden)=>tr.getMeta(predictionKey)??hidden},
  props:{
   handleDOMEvents:{keydown(editor,event){if(event.keyCode===229){editor.dispatch(editor.state.tr.setMeta(predictionKey,true));reset();}return false;},compositionend(){reset();return false;}},
   decorations(state){if(state.doc.textContent||view.composing||predictionKey.getState(state))return null;
    return DecorationSet.create(state.doc,[Decoration.widget(1,()=>{const span=document.createElement('span');span.dataset.ownedPrediction='';span.inert=true;span.setAttribute('aria-hidden','true');span.textContent=text;span.style.cssText='float:inline-start;width:100%;margin-inline-end:-100%';return span;},{side:1,ignoreSelection:true})]);},
  },
 });
}
declare global {interface Window {fixturePrediction(text:string|null):void}}
window.fixturePrediction=text=>view.updateState(view.state.reconfigure({plugins:text?[prediction(text)]:[]}));
declare global {interface Window {fixtureEditor:{set(text:string,start:number,end:number):void;edit(from:number,to:number,text:string):void;appendParagraph():void;select(position:number,end?:number):void;read():string;inspect():unknown}}}
window.fixtureEditor={
 set(text,start,end){const doc=schema.node('doc',null,text.split('\n').map(line=>schema.node('paragraph',null,line?schema.text(line):undefined)));const position=(offset:number)=>offset+1+(text.slice(0,offset).match(/\n/g)?.length??0);view.updateState(EditorState.create({schema,doc,selection:TextSelection.create(doc,position(start),position(end))}));view.focus();},
 edit(from,to,text){view.dispatch(view.state.tr.insertText(text,from+1,to+1));},
 appendParagraph(){view.dispatch(view.state.tr.insert(view.state.doc.content.size,schema.nodes.paragraph.create()));},
 select(position,end=position){view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc,position+1,end+1)));},
 read(){return view.state.doc.textBetween(0,view.state.doc.content.size,'\n');},
 inspect(){const selection=getSelection();return {composing:view.composing,html:view.dom.innerHTML,selection:{from:view.state.selection.from,to:view.state.selection.to},semanticSelection:semanticSelection(view),domSelection:selection?{anchor:{parent:selection.anchorNode?.parentElement?.tagName,offset:selection.anchorOffset,text:selection.anchorNode?.textContent},focus:{parent:selection.focusNode?.parentElement?.tagName,offset:selection.focusOffset,text:selection.focusNode?.textContent}}:null,document:view.state.doc.toJSON()};},
};

// The same native/actor acceptance can exercise an ordinary controlled field.
// Read the browser value and React state separately after native events settle.
declare global {interface Window {fixtureControlledInput():void}}
window.fixtureControlledInput=()=>{
 host.id='prosemirror-richtext';
 const container=document.createElement('div');document.body.prepend(container);
 let rendered='',composing=false,setValue:(text:string)=>void=()=>{throw Error('Owned input not mounted');};
 function ControlledInput(){
  const [value,update]=useState('');setValue=update;rendered=value;
  return createElement('input',{id:'prosemirror',value,'aria-label':'Owned controlled input',
   onChange:(event:React.ChangeEvent<HTMLInputElement>)=>update(event.target.value),
   onCompositionStart:()=>{composing=true;},onCompositionEnd:()=>{composing=false;}});
 }
 const root=createRoot(container);flushSync(()=>root.render(createElement(ControlledInput)));
 const input=container.querySelector('input')!;
 window.fixtureEditor={
  set(text,start,end){flushSync(()=>setValue(text));input.focus();input.setSelectionRange(start,end);},
  edit(from,to,text){
   const next=input.value.slice(0,from)+text+input.value.slice(to);
   // Native value setter bypasses React's value tracker, like a browser edit.
   Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,next);
   input.setSelectionRange(from+text.length,from+text.length);
   input.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertReplacementText',data:text}));
  },
  appendParagraph(){throw Error('Single-line owned field cannot append a paragraph');},
  select(start,end=start){input.focus();input.setSelectionRange(start,end);},
  read(){return input.value;},
  inspect(){return {composing,html:input.outerHTML,controlledValue:rendered,
   semanticSelection:{start:input.selectionStart,end:input.selectionEnd}};},
 };
};

// A second real view permits inline hard breaks, as full rich editors commonly
// do. Read semantic leaf breaks explicitly rather than losing them in textBetween.
const inlineSchema=new Schema({nodes:{doc:{content:'paragraph+'},paragraph:{content:'inline*',group:'block',parseDOM:[{tag:'p'}],toDOM:()=>['p',0]},text:{group:'inline'},hard_break:{inline:true,group:'inline',selectable:false,parseDOM:[{tag:'br'}],toDOM:()=>['br']}}});
const inlineHost=document.createElement('div');inlineHost.id='prosemirror-inline';document.body.append(inlineHost);
const inlineView=new EditorView(inlineHost,{state:EditorState.create({schema:inlineSchema})});
declare global {interface Window {fixtureInlineEditor:Window['fixtureEditor']}}
window.fixtureInlineEditor={
 set(text,start,end){const nodes=text.split('\n').flatMap((line,index)=>[...(index?[inlineSchema.nodes.hard_break.create()]:[]),...(line?[inlineSchema.text(line)]:[])]);const doc=inlineSchema.node('doc',null,[inlineSchema.node('paragraph',null,nodes)]);inlineView.updateState(EditorState.create({schema:inlineSchema,doc,selection:TextSelection.create(doc,start+1,end+1)}));inlineView.focus();},
 edit(from,to,text){inlineView.dispatch(inlineView.state.tr.insertText(text,from+1,to+1));},
 appendParagraph(){inlineView.dispatch(inlineView.state.tr.insert(inlineView.state.doc.content.size,inlineSchema.nodes.paragraph.create()));},
 select(position,end=position){inlineView.dispatch(inlineView.state.tr.setSelection(TextSelection.create(inlineView.state.doc,position+1,end+1)));},
 read(){return inlineView.state.doc.textBetween(0,inlineView.state.doc.content.size,'\n',node=>node.type.name==='hard_break'?'\n':'');},
 inspect(){const selection=getSelection();return {composing:inlineView.composing,html:inlineView.dom.innerHTML,selection:{from:inlineView.state.selection.from,to:inlineView.state.selection.to},semanticSelection:semanticSelection(inlineView),domSelection:selection?{anchor:{parent:selection.anchorNode?.parentElement?.tagName,offset:selection.anchorOffset,text:selection.anchorNode?.textContent},focus:{parent:selection.focusNode?.parentElement?.tagName,offset:selection.focusOffset,text:selection.focusNode?.textContent}}:null,document:inlineView.state.doc.toJSON()};},
};
