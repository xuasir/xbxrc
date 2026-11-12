'use strict';

// 从冻结的官方发布脚本提取原方法，在最小宿主中回放协议和算法。
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const vm = require('node:vm');
const ts = require('typescript');

const evidenceRoot = __dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(evidenceRoot, 'sources.json'), 'utf8'));
const sourceDirectory = process.argv[2];
const inputs = new Map();
const sourceChecks = [];
for (const item of manifest.sources) {
  const filename = sourceDirectory
    ? path.join(sourceDirectory, item.file)
    : item.localRawFile;
  const raw = fs.readFileSync(filename, 'utf8');
  const sha256 = crypto.createHash('sha256').update(raw).digest('hex');
  if (sha256 !== item.sha256) throw new Error('Source hash mismatch: ' + item.file);
  const ast = ts.createSourceFile(item.file, raw, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  if (ast.parseDiagnostics.length) throw new Error('Source parse failed: ' + item.file);
  inputs.set(item.file, { raw, ast, item });
  sourceChecks.push({ file: item.file, sha256, bytes: Buffer.byteLength(raw), result: 'pass' });
}

function walk(node, predicate) {
  if (predicate(node)) return node;
  let found;
  ts.forEachChild(node, child => {
    if (!found) found = walk(child, predicate);
  });
  return found;
}

function moduleBody(input, id) {
  const node = walk(input.ast, candidate =>
    ts.isMethodDeclaration(candidate) && candidate.name.text === String(id));
  if (!node) throw new Error('Module not found: ' + id);
  return node.body;
}

function declaration(input, root, name) {
  for (const node of root.statements) {
    if ((ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node))
      && node.name?.text === name) return node;
    if (ts.isVariableStatement(node)) {
      const found = node.declarationList.declarations.find(item => item.name.text === name);
      if (found) return found;
    }
  }
  throw new Error('Declaration not found: ' + name);
}

function codeOf(input, node) {
  if (!ts.isVariableDeclaration(node)) return node.getText(input.ast);
  let value = node.initializer;
  while (ts.isBinaryExpression(value) && value.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
    value = value.right;
  }
  return 'const ' + node.name.text + ' = ' + value.getText(input.ast) + ';';
}

const core = inputs.get('2871.36f50502.chunk.js');
const coreBody = moduleBody(core, 32871);
const rendererInput = inputs.get('2879.4921609f.chunk.js');
const rendererNode = walk(rendererInput.ast, node => ts.isClassDeclaration(node) && node.name?.text === 'Gi');
const routingInput = inputs.get('8128.1aca8038.chunk.js');
const routingNode = walk(routingInput.ast, node =>
  ts.isClassDeclaration(node) && node.name?.text === 'it'
  && node.getText(routingInput.ast).includes('"/streaming/touchcontrols"'));
if (!rendererNode || !routingNode) throw new Error('Renderer or routing class not found');

const sandbox = { self: { __LOADABLE_LOADED_CHUNKS__: [] }, TextDecoder };
vm.createContext(sandbox);
const run = code => vm.runInContext(code, sandbox, { timeout: 2000 });

run(inputs.get('touch-adaptation-kit-validation.d399c792.chunk.js').raw);
sandbox.validatorExports = {};
sandbox.validatorRequire = id => {
  if (id === 719409) return { LogLevel: { Info: 0 } };
  throw new Error('Unexpected validation dependency: ' + id);
};
sandbox.validatorRequire.d = (out, definitions) => {
  for (const [name, getter] of Object.entries(definitions)) Object.defineProperty(out, name, { get: getter });
};
run('self.__LOADABLE_LOADED_CHUNKS__[0][1][84918]({},validatorExports,validatorRequire);'
  + 'const validator=validatorExports.createJsonValidator({log(){}});');

run('const cc={Start:0,Move:1,End:2};'
  + 'const Mh={Touch:"touch",Mouse:"mouse",Pen:"pen"};'
  + 'const es={Default:"default",Relative:"relative",Absolute:"absolute"};'
  + 'const al={Low:-1,Normal:0,High:1,Max:2};'
  + 'const fo={x:0,y:0},uo=Math.PI/2,ho=2*Math.PI;'
  + 'const pc=class {},Ic=class {},jt={r:0,g:0,b:0,a:0};');
const coreNames = [
  'mo', 'yo', 'vo', '_o', 'bo', 'So', 'ko', 'wo', 'Ao', 'Po', 'Co', 'xo', 'Io',
  'ju', 'Eu', 'Ju', 'Hu', 'Yu', 'Cu', 'xu', 'Bu', 'Fu', 'Lu', 'Nu', 'Uu', 'Ku',
  'Xu', 'Th', 'jo', 'td', 'sl', 'll', 'ul', 'dl', 'hl', 'pl',
  'Ji', 'Hi', 'Yi', 'Ql', 'gs', 'hc', 'Vo', 'Pr',
  'oh', 'sh', 'ah', 'ch',
];
run('const dc=class {};const Kl={Hidden:0,Disabled:1,Idle:2,Toggled:3,Activated:4,Pulled:5};'
  + 'const j={Controller:"Controller",Touch:"Touch",MouseAndKeyboard:"MouseAndKeyboard"};'
  + 'const _r={Blank:"System.Blank-D37853CB-9134-4D40-AE55-EDFBFF9A3368",'
  + 'Standard:"System.Standard-D37853CB-9134-4D40-AE55-EDFBFF9A3368"};'
  + 'const Sr={id:_r.Standard,layoutFile:{content:{}}};const wr=new Map([[_r.Standard,Sr]]);'
  + 'const B={LogLevel:{Info:0,Error:1,Warn:2}},zo={Tag:"Replay/LayoutCoordinator"},vr="system";');
run(codeOf(core, declaration(core, coreBody, 'br')));
for (const name of coreNames) run(codeOf(core, declaration(core, coreBody, name)));
run('Ql.CachedEmptyCalculatedPullAction={pulledAction:undefined,pulledIndex:undefined};');

// 使用发布脚本内同一份 fast-json-patch 方法及 Base64 解码器。
for (const name of [
  'kn', 'wn', 'Pn', 'An', 'Cn', 'xn', 'In', 'Mn', 'Tn', 'On', 'Rn', 'Dn',
  'En', 'Bn', 'Fn', 'Ln', 'zn', 'Nn', 'Un', 'Kn', 'Vn', 'Sn', 'xr',
]) run(codeOf(core, declaration(core, coreBody, name)));
run('const mr=Ln;');
run(core.raw);
sandbox.base64Exports = {};
sandbox.base64Require = { d: sandbox.validatorRequire.d };
run('self.__LOADABLE_LOADED_CHUNKS__.find(item=>item[0].includes(2871))[1][343942]({},base64Exports,base64Require);'
  + 'const decodePatch=patch=>new TextDecoder("utf-8").decode(new Uint8Array(base64Exports.D(patch)));');
run(rendererNode.getText(rendererInput.ast) + ';Gi.TAG="Replay/BabylonRenderer";');
run('const h={r:{Instance:{error(){}}}},u={H:{Instance:{trackEvent(){}}},X:{}};'
  + 'const m={A:value=>JSON.stringify(value)};');
run(codeOf(routingInput, declaration(routingInput, routingNode.parent, 'tt')));
run(routingNode.getText(routingInput.ast));

const records = [];
function check(id, category, observed, matches, note) {
  if (!matches(observed)) throw new Error('Replay mismatch: ' + id + ' ' + JSON.stringify(observed).slice(0, 1600));
  records.push({ id, category, result: 'pass', observed, ...(note ? { note } : {}) });
}
const close = (a, b) => Math.abs(a - b) < 1e-10;
const schemaBase = 'https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/main/touch-adaptation-kit/schemas/';
for (const kind of ['layout', 'context', 'manifest']) {
  sandbox.example = JSON.parse(fs.readFileSync(path.join(evidenceRoot, 'examples', kind + '.json'), 'utf8'));
  sandbox.schemaUrl = schemaBase + kind + '/latest/' + kind + '.json';
  check('schema-' + kind, 'protocol', run('validator.validate(schemaUrl,example,{})'), x => x.success);
}
sandbox.community = JSON.parse(fs.readFileSync(path.join(evidenceRoot, 'examples/forza-layouts.json'), 'utf8')).layouts['layout-1'];
sandbox.schemaUrl = schemaBase + 'layout/latest/layout.json';
sandbox.literalLayout = JSON.parse(fs.readFileSync(path.join(evidenceRoot, 'examples/layout.json'), 'utf8'));
check('community-metadata-wrapper', 'protocol', run('({'
  + 'wrapper:validator.validate(schemaUrl,{name:community.name,author:community.author,content:literalLayout.content},{}),'
  + 'content:validator.validate(schemaUrl,{content:literalLayout.content},{})'
  + '})'), x => !x.wrapper.success && x.content.success,
  'name/author 是社区包装字段；better-xcloud 使用 showLayout 直接注入 layoutFile。');
check('community-forza-strict-schema', 'protocol', run('(() => {'
  + 'const success=validator.validate(schemaUrl,{content:community.content},{}).success;'
  + 'const errors=validator._schemaMap.get(schemaUrl).errors.filter(e=>'
  + 'e.instancePath.endsWith("/knob/stroke")&&e.keyword==="required"&&e.params.missingProperty==="type");'
  + 'return {success,strokeTypeErrors:errors}})()'),
  x => !x.success && x.strokeTypeErrors.length > 0,
  'Forza 原模板的 knob.stroke 省略 type；官方严格 schema 要求该字段。');
check('community-forza-style-normalized', 'protocol', run('(() => {'
  + 'const content=JSON.parse(JSON.stringify(community.content));'
  + 'content.left.outer[2][1].styles.default.knob.stroke.type="solid";'
  + 'return validator.validate(schemaUrl,{content},{})})()'), x => x.success);

run('const routed=[];const handler=Object.create(it.prototype);'
  + 'handler.scope="/streaming/touchcontrols";handler.logger={warning(){},error(){},info(){}};'
  + 'handler.touchControlHandler={onHideTouchControls(){routed.push({command:"hide"})},'
  + 'onUpdateTouchControlState(value){routed.push({command:"patchState",value})},'
  + 'onShowTouchControlLayoutV2(value){routed.push({command:"show",value})},'
  + 'onShowTitleDefaultLayout(){routed.push({command:"showTitleDefault"})}};');
for (const entry of [
  ['/hide', '', 'hide'],
  ['/showlayout', 'gameplay', 'show'],
  ['/showlayoutv2', '{"layoutId":"gameplay","patch":"W10="}', 'show'],
  ['/showlayoutv2', '{"layoutId":""}', 'show'],
  ['/patchstate', '{"patch":"W10="}', 'patchState'],
  ['/showtitledefault', '', 'showTitleDefault'],
]) {
  sandbox.target = handlerTarget(entry[0]);
  sandbox.messageContent = entry[1];
  const observed = run('routed.length=0;handler.onMessage(target,messageContent);routed.slice()');
  check('route-' + entry[0].slice(1) + (entry[1] === '{"layoutId":""}' ? '-empty' : ''),
    'protocol', observed, x => x.length === 1 && x[0].command === entry[2]);
}
function handlerTarget(suffix) { return '/streaming/touchcontrols' + suffix; }
check('route-invalid-layout-id-type', 'protocol',
  run('routed.length=0;handler.onMessage("/streaming/touchcontrols/showlayoutv2",\'{"layoutId":1}\');routed.slice()'),
  x => x.length === 0);
check('route-transaction-cancel', 'protocol',
  run('(() => {let cancelled=0;handler.onTransaction("/streaming/touchcontrols/showlayoutv2","",'
    + '{cancel(){cancelled++}});return {cancelled}})()'), x => x.cancelled === 1);

run('function makeLayoutCoordinator(){const coordinator=Object.create(Vo.prototype);'
  + 'Object.assign(coordinator,{_allowLayoutChanges:true,_scope:"game",_titleId:"game",'
  + '_disableTak:false,_preferredInputModality:"Touch",_allowMouseInteraction:false,'
  + '_perScopeLayoutsStream:{value:new Map(),next(){}},_transformedLayouts:new Map(),'
  + 'takBridge:{onTouchInputActive(){}},logger:{log(){}}});return coordinator}');
check('layout-base-override-default-hide', 'protocol', run('(() => {'
  + 'const c=makeLayoutCoordinator(),titleDefault={id:"title-default"},ids=[];'
  + 'const current=()=>c.recomputeRenderLayout(c._perScopeLayoutsStream.value,titleDefault).id;'
  + 'c.changeLayoutForScope({type:"showLayout",scope:"game",subscope:"base",layout:{id:"base"}});ids.push(current());'
  + 'c.changeLayoutForScope({type:"showLayout",scope:"game",subscope:"override",layout:{id:"override"}});ids.push(current());'
  + 'c.changeLayoutForScope({type:"hide",scope:"game",subscope:"base"});ids.push(current());'
  + 'c.changeLayoutForScope({type:"hide",scope:"game",subscope:"override"});ids.push(current());'
  + 'c.changeLayoutForScope({type:"showTitleDefault",scope:"game",subscope:"base"});ids.push(current());'
  + 'return {ids,blank:br.id,baseIsNull:c._perScopeLayoutsStream.value.get("game").base===null}})()'),
  x => x.ids[0] === 'base' && x.ids[1] === 'override' && x.ids[2] === 'override'
    && x.ids[3] === x.blank && x.ids[4] === 'title-default' && x.baseIsNull);
check('layout-reset-and-empty-id', 'protocol', run('(() => {'
  + 'const c=makeLayoutCoordinator();c.changeLayoutForScope({type:"showLayoutId",scope:"game",subscope:"base",layout:""});'
  + 'const emptyId=c._perScopeLayoutsStream.value.get("game").base.id;'
  + 'c.changeLayoutForScope({type:"reset",scope:"game"});'
  + 'return {emptyId,standardId:Sr.id,entries:c._perScopeLayoutsStream.value.size}})()'),
  x => x.emptyId === x.standardId && x.entries === 0);
check('layer-null-inherits-blank-replaces', 'protocol', run('(() => {'
  + 'const base={right:{inner:[{type:"button",action:"gamepadA"}]}};'
  + 'return {inherited:ch(base,{right:{inner:[null]}}).right.inner[0],'
  + 'replaced:ch(base,{right:{inner:[{type:"blank"}]}}).right.inner[0]}})()'),
  x => x.inherited.action === 'gamepadA' && x.replaced.type === 'blank',
  'layer 数组的 null 继承基础槽位，blank 用作显式空控件覆盖。');

sandbox.encodedPatch = Buffer.from(JSON.stringify([
  { op: 'replace', path: 'fireVisible', value: false },
]), 'utf8').toString('base64');
check('patch-base64-relative-state-and-clone', 'protocol', run('(() => {'
  + 'const original={state:{fireVisible:true,count:1}},errors=[];'
  + 'const result=xr(encodedPatch,original,decodePatch,e=>errors.push(e),e=>errors.push(e));'
  + 'return {decoded:decodePatch(encodedPatch),state:result.state,originalState:original.state,errors:errors.length}})()'),
  x => !x.state.fireVisible && x.originalState.fireVisible && x.state.count === 1 && x.errors === 0);
sandbox.encodedPatch = Buffer.from(JSON.stringify([
  { op: 'replace', path: '/fireVisible', value: false },
  { op: 'replace', path: '/missing', value: 2 },
  { op: 'replace', path: '/count', value: 3 },
]), 'utf8').toString('base64');
check('patch-partial-commit', 'protocol', run('(() => {'
  + 'const original={state:{fireVisible:true,count:1}},fatal=[],partial=[];'
  + 'const result=xr(encodedPatch,original,decodePatch,e=>fatal.push(e),e=>partial.push(e));'
  + 'return {state:result.state,originalState:original.state,fatal:fatal.length,partial:partial.length}})()'),
  x => !x.state.fireVisible && x.state.count === 3 && x.originalState.count === 1
    && x.fatal === 0 && x.partial === 1,
  '单条 patch 失败后继续处理后续操作，成功的更新进入新的 context.state。');
sandbox.encodedPatch = Buffer.from('invalid-json', 'utf8').toString('base64');
check('patch-decode-error-keeps-context', 'protocol', run('(() => {'
  + 'const original={state:{fireVisible:true}},fatal=[];'
  + 'const result=xr(encodedPatch,original,decodePatch,e=>fatal.push(e),()=>{});'
  + 'return {sameContext:result===original,fatal:fatal.length}})()'),
  x => x.sameContext && x.fatal === 1);

check('pointer-delta-and-detach', 'pointer', run('(() => {'
  + 'const events=[],pointer=new Th(0);'
  + 'const control={onPointerDataUpdated(p){events.push({state:p.state,delta:{...p.lastPositionDelta}})}};'
  + 'pointer.updatePointerData(10,20);pointer.tryAttachControl(control);pointer.flushNotification();'
  + 'pointer.updatePointerData(14,18);pointer.flushNotification(true);pointer.detachControl();'
  + 'return {events,pointerState:pointer.state}})()'),
  x => x.events.map(e => e.state).join(',') === '0,1,2'
    && x.events[1].delta.x === 4 && x.events[1].delta.y === -2 && x.pointerState === 1);
check('pointer-move-new-control-start', 'pointer', run('(() => {'
  + 'const events=[],pointer=new Th(0);pointer.updatePointerData(10,20);pointer.updatePointerData(14,18);'
  + 'pointer.tryAttachControl({onPointerDataUpdated(p){events.push({state:p.state,delta:{...p.lastPositionDelta}})}});'
  + 'pointer.flushNotification();return events})()'),
  x => x[0].state === 0 && x[0].delta.x === 0 && x[0].delta.y === 0);
check('collision-circle-rectangle-annulus', 'pointer', run('(() => {'
  + 'const circle=new hl(0,0,10),rect=new pl(0,0,10,20),ring=new ul(5,10,0,0);'
  + 'return {circleEdge:!!circle.getCollidedZone(10,0),circleOutside:!!circle.getCollidedZone(10.1,0),'
  + 'rectangleEdge:!!rect.getCollidedZone(10,20),ringHole:!!ring.getCollidedZone(1,0),'
  + 'ringEdge:!!ring.getCollidedZone(10,0)}})()'),
  x => x.circleEdge && !x.circleOutside && x.rectangleEdge && !x.ringHole && x.ringEdge);
check('collision-start-priority', 'pointer', run('(() => {'
  + 'const container=new sl();container.zones=new Map();'
  + 'const low={canBeInteractedWith(){return true}},high={canBeInteractedWith(){return true}};'
  + 'const a=new ll(),b=new ll(),az=new hl(0,0,10),bz=new hl(0,0,10);bz.priority=al.High;'
  + 'a.addZone(az);b.addZone(bz);container.zones.set(low,a);container.zones.set(high,b);'
  + 'const pointer=new Th(0);pointer.updatePointerData(1,1);container.tryCollision(pointer,1);'
  + 'return {attachedHigh:pointer.control===high}})()'), x => x.attachedHigh);
check('button-disabled-visual-and-layer-action', 'input', run('(() => {'
  + 'function press(action){const commits=[],button=Object.create(Ql.prototype);'
  + 'const state={interactionState:Kl.Disabled};Object.assign(button,{_nonVisualButtonState:Kl.Idle,'
  + '_pullLocation:{x:0,y:0},socket:{unitSize:40},getState(){return state},setState(){},playVibration(){},'
  + 'model:{getProps(){return {enabled:false,visible:true,action}},commitAction(pressed,value){commits.push({pressed,value})}}});'
  + 'button.onPointerStart();button.onStateChangedInternal(state);return commits}'
  + 'const control=Object.create(hc.prototype);control._visible=true;control._compatibleInteractionMode=1;'
  + 'const visibleHit=control.canBeInteractedWith(1);control._visible=false;'
  + 'return {gamepad:press("gamepadA"),layer:press({type:"layer",target:"aim"}),visibleHit,hiddenHit:control.canBeInteractedWith(1)}})()'),
  x => x.gamepad[0].pressed && !x.layer[0].pressed && x.visibleHit && !x.hiddenHit,
  'enabled 控制视觉 Disabled；普通手柄 action 仍输出，layer/layout action 在该状态释放。');

run('function makeInput(){const gamepads=[],mice=[];const provider=new jo('
  + '{onGamepad(value){gamepads.push(value)},onRelativeMouse(value){mice.push(value)},onTouch(){}},'
  + '{changeLayoutForScope(){}},{getSettingsStream(){return {subscribe(fn){fn({touchHapticsEnabled:true})}}}});'
  + 'return {provider,gamepads,mice,blank:{...provider._gamepadModel}}}');
check('input-shared-button-release', 'input', run('(() => {'
  + 'const {provider:p,gamepads:g,blank:z}=makeInput(),a={...z,gamepadA:true};'
  + 'p.startFrame();p.updateGamepadModel(z,a);p.updateGamepadModel(z,a);p.endFrame();'
  + 'p.updateGamepadModel(a,z);const afterFirstRelease=g.length;'
  + 'p.updateGamepadModel(a,z);return {values:g.map(x=>x.gamepadA),afterFirstRelease,count:p._gamepadAccumulator.gamepadA}})()'),
  x => x.values.join(',') === 'true,false' && x.afterFirstRelease === 1 && x.count === 0);
check('input-analog-saturation-release', 'input', run('(() => {'
  + 'const {provider:p,gamepads:g,blank:z}=makeInput(),a={...z,leftJoystickX:0.8};'
  + 'p.startFrame();p.updateGamepadModel(z,a);p.updateGamepadModel(z,a);p.endFrame();'
  + 'const accumulated=p._gamepadAccumulator.leftJoystickX;p.updateGamepadModel(a,z);'
  + 'return {values:g.map(x=>x.leftJoystickX),accumulated}})()'),
  x => close(x.accumulated, 1.6) && close(x.values[0], 1) && close(x.values[1], 0.8));
check('input-opposite-axis-cancel', 'input', run('(() => {'
  + 'const {provider:p,gamepads:g,blank:z}=makeInput();p.startFrame();'
  + 'p.updateGamepadModel(z,{...z,leftJoystickX:1});p.updateGamepadModel(z,{...z,leftJoystickX:-1});p.endFrame();'
  + 'return {last:g[g.length-1].leftJoystickX,commits:g.length}})()'), x => x.last === 0 && x.commits === 1);
check('input-nested-frames', 'input', run('(() => {'
  + 'const {provider:p,gamepads:g,blank:z}=makeInput();p.startFrame();p.startFrame();'
  + 'p.updateGamepadModel(z,{...z,gamepadA:true});p.endFrame();const middle=g.length;p.endFrame();'
  + 'return {middle,final:g.length}})()'), x => x.middle === 0 && x.final === 1);
check('input-relative-mouse-fraction', 'input', run('(() => {'
  + 'const {provider:p,mice:m}=makeInput();'
  + 'for(let i=0;i<3;i++)p.sendRelativeMouseEvent({x:0.4,y:-0.4});'
  + 'return {mice:m,residual:p._mouseAccumulator}})()'),
  x => x.mice.length === 1 && x.mice[0].x === 1 && x.mice[0].y === -1
    && close(x.residual.x, 0.2) && close(x.residual.y, -0.2));
check('input-neutral-epsilon', 'input',
  run('(() => {const {provider:p}=makeInput();return [p.clampAnalogInput(0.000001,-1,1),p.clampAnalogInput(2,-1,1)]})()'),
  x => x[0] === 0 && x[1] === 1);

check('joystick-relative-absolute', 'axis', run('(() => {'
  + 'const stick=Object.create(td.prototype),pointer={lastPositionDelta:{x:8,y:4},position:{x:116,y:92}},'
  + 'prior={x:0.2,y:0.1};stick.socket={x:100,y:80,collisionRadius:60,unitSize:40};'
  + 'stick.model={getProps(){return {relative:true,expand:true}}};stick._settings={relativityMode:"default"};'
  + 'const relative=stick.calculateJoystickValue(pointer,prior);stick._settings.relativityMode="absolute";'
  + 'return {relative,absolute:stick.calculateJoystickValue(pointer,prior),radius:xu(stick.socket,{expand:true})}})()'),
  x => x.radius === 40 && close(x.relative.x, 0.4) && close(x.relative.y, 0.2)
    && close(x.absolute.x, 0.4) && close(x.absolute.y, 0.3));
check('joystick-unit-circle', 'axis', run('yo({x:3,y:4},1)'), x => close(x.x, 0.6) && close(x.y, 0.8));
check('deadzone-axial-observation', 'axis', run('[ju(0,0.1,1),ju(0.5,0.1,1),ju(-0.5,0.1,1),ju(0.01,0.1,1)]'),
  x => x[0] === 0 && close(x[1], 0.55) && close(x[2], -0.55) && close(x[3], 0.109),
  '本发布快照将非零输入映射为 x*(1-d)+sign(x)*d。');
check('deadzone-radial-observation', 'axis', run('Ju({threshold:0.1,radial:true})({x:0.3,y:0.4})'),
  x => close(x.x, 0.33) && close(x.y, 0.44));
for (const kind of ['circular', 'circular-inverse']) {
  sandbox.curveType = kind;
  check('response-curve-' + kind, 'axis-observation', run('(() => {'
    + 'const curve=Hu({type:curveType,range:[0,1]});'
    + 'return {atZero:String(curve(0)),atHalf:String(curve(0.5)),finite:Number.isFinite(curve(0.5))}})()'),
    x => x.atZero === 'NaN' && !x.finite
      && x.atHalf === (kind === 'circular' ? '-Infinity' : 'Infinity'),
    '冻结发布脚本中 Hu 的归一化分母 Eu(range[1],0).y 为 0；这是源码方法回放观察。');
}

const renderArrow = walk(rendererNode, node =>
  ts.isBinaryExpression(node) && ts.isPropertyAccessExpression(node.left)
  && node.left.expression.kind === ts.SyntaxKind.ThisKeyword
  && node.left.name.text === 'render' && ts.isArrowFunction(node.right)).right;
sandbox.renderArrow = renderArrow.getText(rendererInput.ast);
run('function renderFixture(dirty,rootDirty){'
  + 'const calls={viewport:0,texture:0,root:0,leaf:0,groups:0,clear:0};'
  + 'const renderer={engine:{setViewport(){calls.viewport++},clear(){calls.clear++}},'
  + 'spritesGatherer:{updateTexture(){calls.texture++}},rootNode:{update(){calls.root++}},'
  + '_worldMatrix:{},_projectionMatrix:{},_viewport:{},_groups:[{render(){calls.groups++;return true}}],'
  + '_updateNodes:[{parent:{worldMatrix:{},globalOpacity:1,visible:true},update(){calls.leaf++}}],'
  + '_updateNodesIndex:1,_updateWeight:1,_shouldUpdate:dirty,_shouldUpdateRootNode:rootDirty,'
  + '_shouldRender:false,_options:{clearOnRender:false},_renderCountSinceLastLog:0,'
  + '_lastRenderLogTimestamp:Date.now(),_logger:{log(){}},_dispatch(){}};'
  + 'renderer.render=(function(){return eval(renderArrow)}).call(renderer);return {renderer,calls}}');
check('renderer-idle-dispatch', 'render', run('(() => {'
  + 'const {renderer:r,calls}=renderFixture(false,false);r.render();return calls})()'),
  x => x.viewport === 1 && x.texture === 0 && x.root === 0 && x.leaf === 0 && x.groups === 0 && x.clear === 0);
check('renderer-dirty-leaf', 'render', run('(() => {'
  + 'const {renderer:r,calls}=renderFixture(true,false);r.render();return {calls,shouldUpdate:r._shouldUpdate,'
  + 'shouldRender:r._shouldRender,queued:r._updateNodesIndex}})()'),
  x => x.calls.texture === 1 && x.calls.leaf === 1 && x.calls.root === 0
    && x.calls.groups === 1 && !x.shouldUpdate && !x.shouldRender && x.queued === 0);
check('renderer-dirty-root', 'render', run('(() => {'
  + 'const {renderer:r,calls}=renderFixture(true,true);r.render();return calls})()'),
  x => x.root === 1 && x.leaf === 0 && x.groups === 1);
check('renderer-empty-clear', 'render', run('(() => {'
  + 'const {renderer:r,calls}=renderFixture(false,false);r._groups=[];r._shouldRender=true;r.render();return calls})()'),
  x => x.clear === 1 && x.groups === 0);

const result = {
  inspectionDate: manifest.inspectionDate,
  scope: 'Frozen public release methods and compiled validators, with minimal host stubs',
  realBrowserWebGLMeasured: false,
  sourceChecks,
  replayCount: records.length,
  groups: Object.fromEntries([...new Set(records.map(x => x.category))].map(key =>
    [key, records.filter(x => x.category === key).length])),
  observations: [
    '社区 name/author 包装与微软正式 Layout schema 分离',
    'deadzone 对非零值施加输出偏移',
    'responseCurve 的本发布快照方法产生非有限值',
    'rAF 调用持续运行，脏标记决定更新与绘制',
  ],
  records,
};
fs.writeFileSync(path.join(evidenceRoot, 'replay-results.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify({ sources: sourceChecks.length, replays: records.length, groups: result.groups, output: 'replay-results.json' }));
