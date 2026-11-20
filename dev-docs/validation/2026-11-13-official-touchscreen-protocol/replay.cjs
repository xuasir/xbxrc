'use strict';

// 对冻结官方发布脚本的原类、原方法做隔离回放；宿主只提供通道和时钟。
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const ts = require('typescript');
const root = __dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'sources.json'), 'utf8'));
const inputRoot = process.argv[2];
const sources = new Map();
const sourceChecks = [];
const sandbox = {
  self: { __LOADABLE_LOADED_CHUNKS__: [] }, TextEncoder,
  clockMs: 1000.5, performance: { now: () => sandbox.clockMs },
  setInterval: () => 1, clearInterval() {}, setTimeout() {},
  logs: [],
};
vm.createContext(sandbox);
const run = code => vm.runInContext(code, sandbox, { timeout: 4000 });
for (const item of manifest.sources) {
  const filename = inputRoot ? path.join(inputRoot, item.file) : item.localRawFile;
  const raw = fs.readFileSync(filename, 'utf8');
  const sha256 = crypto.createHash('sha256').update(raw).digest('hex');
  assert.equal(sha256, item.sha256, 'Source hash: ' + item.file);
  const ast = ts.createSourceFile(item.file, raw, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  assert.equal(ast.parseDiagnostics.length, 0);
  sources.set(item.file, { raw, ast, item });
  sourceChecks.push({ file: item.file, sha256, bytes: Buffer.byteLength(raw), result: 'pass' });
  run(raw);
}
function walk(node, predicate) {
  if (predicate(node)) return node;
  let found;
  ts.forEachChild(node, child => { if (!found) found = walk(child, predicate); });
  return found;
}
const web = sources.get('web-rtc-stream.d6f6b903.chunk.js');
const webModule = walk(web.ast, n => ts.isMethodDeclaration(n) && n.name.text === '376306');
function classOf(name) {
  const node = webModule.body.statements.find(n => ts.isClassDeclaration(n) && n.name?.text === name);
  assert.ok(node, name);
  return node;
}
function statementContaining(token) {
  return webModule.body.statements.find(n => !ts.isClassDeclaration(n) && n.getText(web.ast).includes(token));
}
function variableOf(name) {
  const statement = webModule.body.statements.find(n => ts.isVariableStatement(n)
    && n.declarationList.declarations.some(d => d.name.text === name));
  return statement.declarationList.declarations.find(d => d.name.text === name);
}

// 原 CircularBuffer 与输入领域模型；polyfill require 在 Node 中为空操作。
run(`const moduleTable=Object.assign({},...self.__LOADABLE_LOADED_CHUNKS__.map(x=>x[1]));
  const cache={};
  const define=(obj,key,value)=>Object.defineProperty(obj,key,{value,writable:true,enumerable:true,configurable:true});
  const logger={info(...args){logs.push(args)},warning(...args){logs.push(args)},verbose(){},error(...args){logs.push(args)},throw(message){throw new Error(message)}};
  function req(id){
    if(id===258212)return define;
    if(id!==997406&&id!==791211&&id!==82361)return {};
    if(cache[id])return cache[id].exports;
    const module=cache[id]={exports:{}};
    moduleTable[id](module,module.exports,req);
    return module.exports;
  }
  req.d=(out,definitions)=>{for(const [key,get] of Object.entries(definitions))Object.defineProperty(out,key,{get,enumerable:true})};
  req.n=value=>()=>value;
  const c=req(997406),l=req(791211),s=()=>define,a=()=>Object.assign;
  const u={r:{Instance:logger}},b={ws:{BadLength:1,InvalidArgument:2,Unsupported:3,InvalidState:4,BadConfiguration:5}};
  const U={A:value=>String(value)},G=1,_=2;
  var B,z,N,q,H,X,Q,j;`);
run(statementContaining('PointerDown').getText(web.ast));
run(statementContaining('ChannelProtocolV1="1.0"').getText(web.ast));
for (const name of ['O', 'I', 'W', 'K', 'Y']) run(classOf(name).getText(web.ast));
run('const J=' + variableOf('J').initializer.getText(web.ast));
run(`function channel(version=9,protocol="2.0") {
  return {version,protocol,sent:[],isOpen:()=>true,send(data){this.sent.push(Array.from(data))},
    setOpenedCallback(callback){this.opened=callback},setMessageReceivedCallback(callback){this.received=callback},close(){}};
}
const element={scrollWidth:1280,scrollHeight:720};
const reporter=new O();
const event=(type="pointerdown",id=7)=>({type,pointerId:id,x:320,y:180,width:20,height:10,pressure:0.5,twist:45,clientWidth:1280,clientHeight:720});
const freshV2=()=>new Y(channel(),channel(),element,{});
const decodePointer=(data,offset=0)=>{const view=new DataView(Uint8Array.from(data).buffer);return {
  height:view.getUint16(offset,true),width:view.getUint16(offset+2,true),pressure:view.getUint8(offset+4),
  twist:view.getUint16(offset+5,true),pointerId:view.getUint32(offset+7,true),x:view.getUint32(offset+11,true),
  y:view.getUint32(offset+15,true),type:view.getUint8(offset+19)}};
const decodeState=(data,offset=0)=>{const view=new DataView(Uint8Array.from(data).buffer);return {
  pointerId:view.getUint32(offset,true),upDown:view.getUint8(offset+4),move:view.getUint8(offset+5),
  height:view.getUint16(offset+6,true),width:view.getUint16(offset+8,true),pressure:view.getUint8(offset+10),
  twist:view.getUint16(offset+11,true),x:view.getUint32(offset+13,true),y:view.getUint32(offset+17,true)}};
const hex=data=>Array.from(data,value=>value.toString(16).padStart(2,"0")).join(" ");
const legacy=(events,version=8)=>Array.from(reporter.generateInputReport(version,clockMs,1,1080,1920,undefined,undefined,[{events}]));
const unreliable=(events,version=9)=>{const ch=freshV2();ch.updateCurrentPointerInputFrame(events);return Array.from(reporter.generateUnreliableInputFrameReport(version,1,ch.currentFrameState.calculateChanges(ch.acknowledgedFrameState),1080,1920))};`);

const records = [];
function check(id, code, expected, note) {
  const observed = JSON.parse(JSON.stringify(run(code)));
  if (typeof expected === 'function') assert.ok(expected(observed), id + ': ' + JSON.stringify(observed));
  else assert.deepEqual(observed, expected, id);
  records.push({ id, result: 'pass', observed, ...(note ? { note } : {}) });
}
check('report-flags', '({pointer:c.$B.PointerReport,clientMetadata:c.$B.ClientMetadata,serverMetadata:c.$B.ServerMetadata,state:c.$B.UnreliableInputReport,ack:c.$B.UnreliableInputAck})',
  { pointer:4,clientMetadata:8,serverMetadata:16,state:512,ack:1024 });
check('header-versions', '[6,7,8,9,10].map(v=>({version:v,size:reporter.calculateHeaderSize(v)}))',
  [{version:6,size:5},{version:7,size:13},{version:8,size:14},{version:9,size:14},{version:10,size:14}]);
for (const version of [6,7,8]) {
  check('header-bytes-v'+version, `(()=>{const buffer=new ArrayBuffer(16);const size=reporter.generateHeader(${version},4,0x12345678,new DataView(buffer));return {size,hex:hex(new Uint8Array(buffer,0,size))}})()`,
    value=>value.size===(version===6?5:version===7?13:14)&&value.hex.startsWith(version===8?'04 00 78 56 34 12':'04 78 56 34 12'));
}
check('client-metadata-and-cap', `(()=>{const ch=freshV2();ch.maxTouchPoints=999;ch.sendClientMetadataReport(ch.reliableChannel);const packet=ch.reliableChannel.sent[0];return {length:packet.length,mask:packet.slice(0,2),maxTouchPoints:packet.at(-1)}})()`,
  {length:15,mask:[8,0],maxTouchPoints:255});
check('client-metadata-version-gate', `(()=>{const ch=new K(channel(2,"1.0"),element,{});ch.sendClientMetadataReport(ch.dataChannel);return ch.dataChannel.sent.length})()`, 0);
check('input-token-wrap', `(()=>{const r=new O();r.inputTokenCounter=0xfffffffe;const a=Array.from(r.generateClientMetadataReport(8,10));const b=Array.from(r.generateClientMetadataReport(8,10));return [a,b].map(p=>new DataView(Uint8Array.from(p).buffer).getUint32(2,true))})()`, [4294967295,0]);
check('legacy-single-pointer', '(()=>{const p=legacy([event()]);return {length:p.length,frameCount:p[14],eventCount:p[15],pointer:decodePointer(p,16),hex:hex(p)}})()',
  p=>p.length===36&&p.frameCount===1&&p.eventCount===1&&JSON.stringify(p.pointer)===JSON.stringify({height:15,width:30,pressure:127,twist:45,pointerId:7,x:480,y:270,type:1}));
check('legacy-three-event-types', '["pointerdown","pointerup","pointermove"].map(t=>decodePointer(legacy([event(t)]),16).type)', [1,2,3]);
check('legacy-multi-frame-two-fingers', `(()=>{const frames=[{events:[event("pointerdown",7),event("pointerdown",9)]},{events:[event("pointermove",7)]}];const p=Array.from(reporter.generateInputReport(8,clockMs,2,1080,1920,undefined,undefined,frames));return {length:p.length,frameCount:p[14],counts:[p[15],p[56]],ids:[decodePointer(p,16).pointerId,decodePointer(p,36).pointerId,decodePointer(p,57).pointerId]}})()`,
  {length:77,frameCount:2,counts:[2,1],ids:[7,9,7]});
check('legacy-up-keeps-fields', 'decodePointer(legacy([event("pointerup")]),16)',
  {height:15,width:30,pressure:127,twist:45,pointerId:7,x:480,y:270,type:2});
check('client-dimensions-default', `(()=>{const e=event();e.clientWidth=0;e.clientHeight=111;const p=legacy([e]);return {clientWidth:e.clientWidth,clientHeight:e.clientHeight,pointer:decodePointer(p,16)}})()`,
  value=>value.clientWidth===1280&&value.clientHeight===720&&value.pointer.x===480&&value.pointer.y===270);
check('integer-truncation-and-wrap', `(()=>{const e=event();e.x=-1;e.y=10.99;e.width=50000;e.pressure=2;return decodePointer(legacy([e]),16)})()`,
  {height:15,width:9464,pressure:254,twist:45,pointerId:7,x:4294967295,y:16,type:1},
  '官方序列化器沿 DataView 转换：有限小数向零截断，越界无符号整数按字段位宽回绕。');
check('normalized-tak-pointer', `(()=>{const e=event();Object.assign(e,{x:0.25,y:0.5,width:0.02,height:0.02,clientWidth:1,clientHeight:1});return decodePointer(legacy([e]),16)})()`,
  {height:21,width:38,pressure:127,twist:45,pointerId:7,x:480,y:540,type:1});
check('unreliable-single-pointer-v9', '(()=>{const p=unreliable([event()]);return {length:p.length,frameId:new DataView(Uint8Array.from(p).buffer).getUint32(14,true),gamepads:p[18],pointers:p[19],pointer:decodeState(p,20),tail:p.slice(41),hex:hex(p)}})()',
  p=>p.length===44&&p.frameId===1&&p.gamepads===0&&p.pointers===1&&JSON.stringify(p.pointer)===JSON.stringify({pointerId:7,upDown:1,move:0,height:15,width:30,pressure:127,twist:45,x:480,y:270})&&p.tail.join(',')==='0,0,0');
check('unreliable-single-pointer-v10', '(()=>{const p=unreliable([event()],10);return {length:p.length,tail:p.slice(41)}})()', {length:45,tail:[0,0,0,0]});
check('unreliable-two-fingers', '(()=>{const p=unreliable([event("pointerdown",7),event("pointerdown",9)]);return {length:p.length,count:p[19],ids:[decodeState(p,20).pointerId,decodeState(p,41).pointerId]}})()', {length:65,count:2,ids:[7,9]});
check('counter-sequence', `(()=>{const ch=freshV2();return ["pointerdown","pointermove","pointermove","pointerup","pointerdown","pointermove"].map(type=>{ch.updateCurrentPointerInputFrame([event(type)]);const p=ch.currentFrameState.pointers[0];return {upDown:p.pointerStateCountByStateType[0],move:p.pointerStateCountByStateType[1],up:p.isPointerUp()}})})()`,
  [{upDown:1,move:0,up:false},{upDown:1,move:1,up:false},{upDown:1,move:1,up:false},{upDown:2,move:1,up:true},{upDown:3,move:1,up:false},{upDown:3,move:2,up:false}]);
check('duplicate-down-up', `(()=>{const ch=freshV2();for(const type of ["pointerdown","pointerdown","pointerup","pointerup"])ch.updateCurrentPointerInputFrame([event(type)]);const p=ch.currentFrameState.pointers[0];return {counts:Array.from(p.pointerStateCountByStateType),discriminator:p.discriminator}})()`, {counts:[2,0],discriminator:4});
check('counter-wrap', `(()=>{const ch=freshV2();ch.updateCurrentPointerInputFrame([event()]);const p=ch.currentFrameState.pointers[0];p.pointerStateCountByStateType.set([255,255]);ch.updateCurrentPointerInputFrame([event("pointerup"),event("pointermove")]);return {counts:Array.from(p.pointerStateCountByStateType),up:p.isPointerUp()}})()`, {counts:[0,0],up:true});
check('unacknowledged-up-retained', `(()=>{const ch=freshV2();ch.updateCurrentPointerInputFrame([event(),event("pointerup")]);const changes=ch.currentFrameState.calculateChanges(ch.acknowledgedFrameState);const p=changes.changedPointers[0];return {count:changes.changedPointers.length,counts:Array.from(p.pointerStateCountByStateType),x:p.x,y:p.y}})()`, {count:1,counts:[2,0],x:320,y:180});
check('ack-baseline-and-retransmission', `(()=>{clockMs=1000;const ch=freshV2();ch.serverWidth=1920;ch.serverHeight=1080;ch.updateCurrentPointerInputFrame([event()]);ch.sendGamepadInput(0,[]);clockMs=1008;ch.sendGamepadInput(0,[]);clockMs=1016;ch.sendGamepadInput(0,[]);const sentBeforeAck=ch.dataChannel.sent.length;const ids=ch.dataChannel.sent.map(p=>new DataView(Uint8Array.from(p).buffer).getUint32(14,true));ch.processMessage(Uint8Array.from([0,4,1,0,0,0]).buffer);clockMs=1032;ch.sendGamepadInput(0,[]);return {sentBeforeAck,ids,retransmits:ch.inputFrameRetransmitCount,baseline:ch.acknowledgedFrameState.frameId,queueLength:ch.ackQueue.length,sentAfterAck:ch.dataChannel.sent.length}})()`,
  {sentBeforeAck:2,ids:[1,1],retransmits:1,baseline:1,queueLength:0,sentAfterAck:2});
check('cumulative-and-duplicate-ack', `(()=>{clockMs=2000;const ch=freshV2();for(const type of ["pointerdown","pointermove","pointerup"]){ch.updateCurrentPointerInputFrame([event(type)]);ch.sendGamepadInput(0,[]);clockMs+=20;}ch.ackFrame(2);const first={baseline:ch.acknowledgedFrameState.frameId,queue:ch.ackQueue.length};ch.ackFrame(1);ch.ackFrame(2);const duplicate={baseline:ch.acknowledgedFrameState.frameId,queue:ch.ackQueue.length};ch.ackFrame(3);return {first,duplicate,final:ch.acknowledgedFrameState.frameId,changed:ch.currentFrameState.calculateChanges(ch.acknowledgedFrameState).changedPointers.length}})()`,
  {first:{baseline:2,queue:1},duplicate:{baseline:2,queue:1},final:3,changed:0});
check('server-metadata-v6-v9', `(()=>{return [6,9].map(version=>{const ch=new K(channel(version,"1.0"),element,{});const h=version>=8?2:1;const p=new ArrayBuffer(h+8);const v=new DataView(p);if(h===2)v.setUint16(0,16,true);else v.setUint8(0,16);v.setUint32(h,1080,true);v.setUint32(h+4,1920,true);ch.processMessage(p);return {version,length:p.byteLength,width:ch.serverWidth,height:ch.serverHeight,received:ch.hasReceivedServerMetadata}})})()`,
  [{version:6,length:9,width:1920,height:1080,received:true},{version:9,length:10,width:1920,height:1080,received:true}]);
check('legacy-metadata-gate', `(()=>{const ch=new K(channel(8,"1.0"),element,{});ch.queuePointerInput(0,[event()]);const before=ch.dataChannel.sent.length;ch.processMessage(Uint8Array.from([16,0,56,4,0,0,128,7,0,0]).buffer);ch.queuePointerInput(0,[event()]);return {before,after:ch.dataChannel.sent.length}})()`, {before:0,after:1});
check('ack-queue-growth', `(()=>{const ch=freshV2();clockMs=3000;for(let i=0;i<121;i++){const e=event(i===0?"pointerdown":"pointermove");e.x=i;ch.updateCurrentPointerInputFrame([e]);ch.sendGamepadInput(0,[]);clockMs+=20;}return {length:ch.ackQueue.length,capacity:ch.ackQueue.maxLength,frameId:ch.currentFrameState.frameId}})()`, {length:121,capacity:240,frameId:121});
check('channel-factory', '({legacy:J(channel(8,"1.0"),element,{}).constructor.name,state:J(channel(9,"2.0"),element,{},channel()).constructor.name,versions:{legacy:[H.MinVersion,H.MaxVersion],state:[X.MinVersion,X.MaxVersion]}})',
  {legacy:'K',state:'Y',versions:{legacy:['1','10'],state:['9','10']}});

// 通道创建方法原文回放；ce 是用于记录配置的最小 WebRTC 包装宿主。
const streamClass = classOf('ue');
const setupNodes = streamClass.members.filter(n => ['setupUnreliableInputModelDataChannels','setupLegacyInputDataChannel'].includes(n.name?.text));
run(`class ce {constructor(native){Object.assign(this,native)}setInternalOpenedCallback(callback){this.opened=callback}};
  const setup={${setupNodes.map(n=>n.getText(web.ast)).join(',')}};`);
check('data-channel-configurations', `(()=>{const created=[];const owner={logger,onWebRtcConnectionStateChanged(){}};const peer={createDataChannel(name,options){created.push({name,...options});return {name}}};setup.setupUnreliableInputModelDataChannels.call(owner,peer,()=>{},()=>{});setup.setupLegacyInputDataChannel.call(owner,peer,()=>{},()=>{});return created})()`,
  [{name:'unreliableinput',protocol:'2.0',ordered:false,maxRetransmits:0},{name:'reliableinput',protocol:'2.0',ordered:true},{name:'input',protocol:'1.0',ordered:true}]);

// 保存原文与 AST 排版版本，回放始终使用原文；链接以排版文件行号为准。
const excerpts = [
  ['input-reporter',web,[classOf('O'),statementContaining('PointerDown')]],
  ['input-channels',web,['I','W','K','Y'].map(classOf)],
  ['channel-negotiation',web,[statementContaining('ChannelProtocolV1="1.0"'),variableOf('J'),
    ...streamClass.members.filter(n=>['setupUnreliableInputModelDataChannels','setupLegacyInputDataChannel','handleSdpAnswer'].includes(n.name?.text)),
    walk(streamClass,n=>ts.isVariableDeclaration(n)&&n.initializer?.getText(web.ast).includes('useUnreliableInput:this.inputConfiguration.useUnreliableInput'))].filter(Boolean)],
];
const domain=sources.get('8128.1aca8038.chunk.js');
excerpts.push(['input-state-model',domain,[walk(domain.ast,n=>ts.isMethodDeclaration(n)&&n.name.text==='997406')]]);
const utils=sources.get('2091.34346f46.chunk.js');
excerpts.push(['circular-buffer',utils,[walk(utils.ast,n=>ts.isClassExpression(n)
  &&n.members.some(member=>member.name?.text==='getNextAvailableElem'))]]);
const excerptRoot=path.join(root,'source-excerpts');
fs.mkdirSync(excerptRoot,{recursive:true});
const printer=ts.createPrinter({newLine:ts.NewLineKind.LineFeed});
const excerptIndex=[];
for(const [name,source,nodes] of excerpts){
  const filename=name+'.txt';
  const header='// 来源 '+source.item.file+'；原文 SHA-256 '+source.item.sha256+'\n';
  const formatted=header+nodes.map(node=>{
    const text=printer.printNode(ts.EmitHint.Unspecified,ts.getSynthesizedDeepClone(node),source.ast);
    if(ts.isMethodDeclaration(node))return '({\n'+text+'\n});';
    if(ts.isVariableDeclaration(node))return 'const '+text+';';
    if(ts.isClassExpression(node))return 'const CircularBuffer = '+text+';';
    return text;
  }).join('\n\n')+'\n';
  const raw=nodes.map(node=>node.getText(source.ast)).join('\n');
  assert.equal(ts.createSourceFile(name,formatted,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS).parseDiagnostics.length,0,'Formatted excerpt parses: '+name);
  fs.writeFileSync(path.join(excerptRoot,filename),formatted);
  excerptIndex.push({file:filename,source:source.item.file,rawSha256:crypto.createHash('sha256').update(raw).digest('hex'),formattedSha256:crypto.createHash('sha256').update(formatted).digest('hex'),nodes:nodes.map(node=>({originalStart:node.getStart(source.ast),originalEnd:node.end}))});
}
fs.writeFileSync(path.join(excerptRoot,'index.json'),JSON.stringify(excerptIndex,null,2)+'\n');
const output={inspectionDate:'2026-11-13',method:'官方原类/方法隔离回放，真实 CircularBuffer，模拟通道与时钟',sourceChecks,passed:records.length,failed:0,records};
fs.writeFileSync(path.join(root,'replay-results.json'),JSON.stringify(output,null,2)+'\n');
console.log(JSON.stringify({passed:records.length,failed:0,sources:sourceChecks.length,excerpts:excerptIndex.length}));
