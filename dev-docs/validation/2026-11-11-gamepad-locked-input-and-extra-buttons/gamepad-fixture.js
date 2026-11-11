import { defaultGameControls } from '/src/shared/gamepad/preferences.ts'
import { DEFAULT_KEYBOARD_MAPPING } from '/src/shared/gamepad/keyboard-fallback-bridge.ts'
import { gamepadConfig, inputController, saveGamepadConfig } from '/src/shared/gamepad/runtime.ts'
import { setUiLocale } from '/src/i18n/index.ts'
import { findNextFocusable } from '/src/navigation/core/pathfinding.ts'
const pad = { index:0,id:'Xbox validation controller',connected:true,mapping:'standard',buttons:Array.from({length:21},()=>({value:0,pressed:false})),axes:[0,0,0,0,0] }
const other = { index:1,id:'Raw rear-button controller',connected:false,mapping:'',buttons:Array.from({length:25},()=>({value:0,pressed:false})),axes:[0,0,0,0,-1] }
Object.defineProperty(navigator,'getGamepads',{value:()=>[pad.connected?pad:null,other.connected?other:null],configurable:true})
const results=[]
let menuRequests=0
inputController.onMenuRequested(()=>menuRequests++)
const toolbar=document.createElement('div')
toolbar.style.cssText='position:fixed;top:4px;right:4px;z-index:2147483000;display:flex;flex-wrap:wrap;max-width:calc(100vw - 8px);gap:4px;padding:4px;background:#18202d;color:white;border-radius:8px;font:11px sans-serif'
document.body.append(toolbar)
const wait=ms=>new Promise(r=>setTimeout(r,ms))
async function until(test,label){for(let i=0;i<160;i++){if(test())return;await wait(50)}throw Error('timeout: '+label)}
function check(name,value,detail={}){results.push({name,pass:!!value,detail});document.getElementById('gamepad-results').textContent=JSON.stringify(results);if(!value)throw Error(name)}
function click(id){const el=document.getElementById(id);if(!el||el.disabled)throw Error('unavailable '+id);el.click()}
async function pressed(indices,device=pad){device.buttons.forEach((b,i)=>{b.value=indices.includes(i)?1:0;b.pressed=b.value===1});await wait(130)}
async function key(type,code,modifiers={}){window.dispatchEvent(new KeyboardEvent(type,{key:code,code,bubbles:true,cancelable:true,...modifiers}));await wait(100)}
async function activateKeyboard(){await key('keydown','KeyQ');await key('keyup','KeyQ')}
async function activatePad(device=pad){await pressed([3],device);await pressed([],device)}
async function escape(){await key('keydown','Escape',{ctrlKey:true,shiftKey:true});await key('keyup','Escape',{ctrlKey:true,shiftKey:true})}
const recording=()=>document.querySelector('.pad-editor')?.dataset.recording
const locked=()=>!!document.querySelector('[data-input-locked=true]')
async function page(tool){location.hash='/controller/'+tool;await until(()=>!!document.getElementById(tool==='test'?'setting.gamepad.editor.lockStatus':'setting.gamepad.editor.physical.south'),'page '+tool);await wait(300)}
async function capture(target,indices,device=pad){click('setting.gamepad.editor.'+(target.includes('Stick')?'direction.':'physical.')+target);await until(recording,'capture');await pressed(indices,device);check('input held for '+target,recording()===target&&inputController.getSnapshot().exclusiveCapture);await pressed([],device);await until(()=>!recording(),'release');await wait(100);check('automatic release and focus for '+target,document.getElementById('setting.gamepad.editor.'+(target.includes('Stick')?'direction.':'physical.')+target)?.dataset.focused==='true')}
async function save(){await until(()=>!!document.getElementById('setting.gamepad.editor.apply')&&!document.getElementById('setting.gamepad.editor.apply').disabled,'apply enabled');click('setting.gamepad.editor.apply');await until(()=>!document.querySelector('.pad-editor'),'save');await wait(300)}
function labelsSeparated(){const boxes=[...document.querySelectorAll('.gamepad-mapping__callout')].map(e=>e.getBoundingClientRect());return boxes.every((a,i)=>boxes.every((b,j)=>i===j||a.right<=b.left||b.right<=a.left||a.bottom<=b.top||b.bottom<=a.top))}
function geometry(){const nodes=[...document.querySelectorAll('.gamepad-tool-page [data-focusable]')].filter(e=>!e.disabled&&e.getBoundingClientRect().width);const first=document.getElementById('setting.gamepad.editor.physical.south');const reached=new Set([first]);const queue=[first];while(queue.length){const n=queue.shift();for(const d of ['up','down','left','right']){const next=findNextFocusable(n,nodes,d);if(next&&!reached.has(next)){reached.add(next);queue.push(next)}}}return {width:innerWidth,height:innerHeight,nodes:nodes.length,unreachable:nodes.filter(n=>!reached.has(n)).map(n=>n.id),overlap:!labelsSeparated(),overflow:document.documentElement.scrollWidth>innerWidth}}
async function run(){results.length=0;menuRequests=0;try{
 await until(()=>!!document.getElementById('controller.tabs.settings'),'controller');await pressed([]);other.connected=false;pad.connected=true;setUiLocale('zh')
 await saveGamepadConfig({gamepad_control_source:'gamepad',gamepad_game_controls:defaultGameControls(),gamepad_keyboard_mapping:structuredClone(DEFAULT_KEYBOARD_MAPPING)})
 click('controller.tabs.settings');await wait(300)
 check('single mapping entry and always available test',!!document.getElementById('controller.settings.layout')&&!document.getElementById('controller.settings.keyboard')&&!document.getElementById('controller.settings.test').disabled)
 click('controller.settings.layout');await until(()=>!!document.querySelector('.pad-source-badge'),'mapping page');await wait(300)
 check('Xbox output title and compact automatic source badge',document.querySelector('h1').textContent.includes('Xbox 输出映射')&&document.querySelector('.gamepad-tool-page__title-row .pad-source-badge').textContent.includes(pad.id)&&!document.querySelector('.pad-source-picker'))
 check('20px title with top actions',getComputedStyle(document.querySelector('.gamepad-tool-page h1')).fontSize==='20px'&&!!document.querySelector('.gamepad-tool-page__header [id="setting.gamepad.editor.apply"]')&&!document.querySelector('.gamepad-tool-page__footer'))
 check('17 targets and 8 direct stick directions',document.querySelectorAll('.gamepad-mapping__callout').length===17&&document.querySelectorAll('.gamepad-mapping__direction').length===8)
 check('callouts stay separated',labelsSeparated())
 await capture('south',[1]);check('B is input for output A',document.getElementById('setting.gamepad.editor.physical.south').textContent.includes('B')&&location.hash==='#/controller/layout')
 check('capture stays in original diagram',!document.querySelector('.gamepad-diagram-editor')&&!document.querySelector('[role=dialog]'))
 await capture('home',[4,5]);check('chord labels appear on Xbox target',document.getElementById('setting.gamepad.editor.physical.home').textContent.includes('LB + RB'))
 await capture('south',[17]);check('rear input shown on output A',document.getElementById('setting.gamepad.editor.physical.south').textContent.includes('扩1'))
 await activateKeyboard();check('toggle shows enable action when disabled',document.getElementById('setting.gamepad.editor.enabled').textContent.includes('开启'))
 click('setting.gamepad.editor.enabled');await wait(100);check('toggle shows disable action when enabled',document.getElementById('setting.gamepad.editor.enabled').textContent.includes('关闭'))
 click('setting.gamepad.editor.physical.south');await until(recording,'keyboard capture');await key('keydown','KeyP');check('keyboard held keeps recording',recording()==='south');await key('keyup','KeyP');await until(()=>!recording(),'keyboard release');check('keyboard release restores A focus',document.getElementById('setting.gamepad.editor.physical.south').dataset.focused==='true')
 click('setting.gamepad.editor.physical.home');await until(recording,'keyboard chord');await key('keydown','KeyO');await key('keydown','KeyL');await key('keyup','KeyO');check('chord waits for all keys released',recording()==='home');await key('keyup','KeyL');await until(()=>!recording(),'keyboard chord release');check('keyboard chord shown on Xbox',document.getElementById('setting.gamepad.editor.physical.home').textContent.includes('O + L'))
 click('setting.gamepad.editor.direction.leftStickUp');await until(recording,'keyboard direction');await key('keydown','KeyR');await key('keyup','KeyR');await until(()=>!recording(),'keyboard direction release');check('direction restores original focus',document.getElementById('setting.gamepad.editor.direction.leftStickUp').dataset.focused==='true')
 await activatePad();check('switch preserves gamepad draft',document.getElementById('setting.gamepad.editor.physical.home').textContent.includes('LB + RB'))
 await save();check('both sources persist independently',gamepadConfig().gamepad_keyboard_mapping.chords?.[0]?.control==='home'&&inputController.getSnapshot().preferences.gameControls.deviceMappings.length===1)
 await page('layout');check('saved chord reloads',document.getElementById('setting.gamepad.editor.physical.home').textContent.includes('LB + RB'))
 other.connected=true;await until(()=>inputController.getSnapshot().devices.length===2,'raw device');await activatePad(other)
 check('nonstandard source begins unbound',document.getElementById('setting.gamepad.editor.physical.south').textContent.includes('未绑定'))
 await capture('south',[24],other);check('nonstandard button index 24 maps to A',document.getElementById('setting.gamepad.editor.physical.south').textContent.includes('24'))
 click('setting.gamepad.editor.direction.leftStickRight');await until(recording,'raw axis');other.axes[2]=-.75;await wait(200);check('axis held keeps recording',recording()==='leftStickRight');other.axes[2]=0;await wait(250);check('axis completes and label updates',!recording()&&document.getElementById('setting.gamepad.editor.direction.leftStickRight').getAttribute('aria-label').includes('2'))
 click('setting.gamepad.editor.physical.l2');await until(recording,'trigger axis');other.axes[4]=1;await wait(200);other.axes[4]=-1;await wait(250);check('full trigger axis captures from rest -1',!recording()&&document.getElementById('setting.gamepad.editor.physical.l2').textContent.includes('4'))
 await save();check('two device profiles survive save',inputController.getSnapshot().preferences.gameControls.deviceMappings.length===2)
 await page('layout');await activatePad(other);click('setting.gamepad.editor.defaults');await save();check('reset affects selected source only',inputController.getSnapshot().preferences.gameControls.deviceMappings.find(p=>p.layout.includes('Xbox validation'))?.bindings.some(b=>b.target==='home'&&b.kind==='buttons'&&b.buttons.length===2))
 await until(()=>!!document.getElementById('controller.settings.test'),'test entry');click('controller.settings.test');await until(locked,'test lock');await wait(300);check('test locks with compact badge, touch return and no top banner',locked()&&!document.querySelector('.pad-source-picker')&&!document.querySelector('.gamepad-lock-status')&&!!document.getElementById('setting.gamepad.editor.touchBack'));check('unknown extras start empty',document.querySelectorAll('[data-raw-button]').length===0)
 await key('keydown','KeyP');check('test auto detects keyboard A',document.querySelector('.pad-source-badge').textContent.includes('键盘')&&document.querySelector('[data-mode=input] [data-button=south]').parentElement.textContent.includes('1.00'));await key('keyup','KeyP')
 await key('keydown','KeyR');check('keyboard stick appears in test',document.querySelector('.gamepad-readings__sticks').textContent.includes('-1.00'));await key('keyup','KeyR')
 await pressed([17]);check('test auto detects physical rear input and mapped Xbox A',document.querySelector('.pad-source-badge').textContent.includes(pad.id)&&document.querySelector('[data-mode=input] [data-button=south]').parentElement.textContent.includes('1.00'))
 check('only detected rear input appears and highlights',document.querySelectorAll('[data-raw-button]').length===1&&document.querySelector('[data-raw-button="17"]').dataset.pressed==='true');check('UI and stream neutral during testing',Object.values(inputController.getSnapshot().uiState.buttons).every(v=>v===0)&&Object.values(inputController.getSnapshot().gameState.buttons).every(v=>v===0))
 await pressed([24],other);check('test auto detects raw controller and exposes all raw inputs',document.querySelector('.pad-source-badge').textContent.includes(other.id)&&document.querySelector('[data-raw-button="24"]').dataset.pressed==='true')
 await pressed([],other);await pressed([]);pad.connected=false;other.connected=false;await wait(200);await key('keydown','KeyP');check('keyboard test works with hardware disconnected',locked()&&document.querySelector('.pad-source-badge').textContent.includes('键盘'));await key('keyup','KeyP');click('setting.gamepad.editor.touchBack');await until(()=>!locked(),'touch test exit');await wait(300)
 check('test exit clears observers and restores entry focus',!inputController.getSnapshot().keyboardObserving&&document.getElementById('controller.settings.test').dataset.focused==='true')
 pad.connected=true;await pressed([]);await page('test');await pressed([8,9]);await wait(1050);await until(()=>!locked(),'combo exit');await pressed([]);await wait(250);check('physical Menu/View long hold exits auto test',!inputController.getSnapshot().exclusiveCapture)
 await page('layout');await activatePad();const geom=geometry();check('geometric navigation reaches all controls',geom.nodes>=44&&geom.unreachable.length===0,geom)
 check('global menu suppressed during capture and test',menuRequests===0,{menuRequests})
 toolbar.querySelector('[data-result]').textContent=results.length+'项通过'
 }catch(error){results.push({name:'execution error',pass:false,error:String(error)});toolbar.querySelector('[data-result]').textContent=String(error)}document.getElementById('gamepad-results').textContent=JSON.stringify(results,null,2)}
function button(label,action){const b=document.createElement('button');b.textContent=label;b.style.cssText='font:inherit;padding:4px 6px;white-space:nowrap';b.onclick=action;toolbar.append(b)}
button('运行回归',run);button('映射页',()=>page('layout'));button('测试页',()=>page('test'));button('结束测试',escape)
button('深色',()=>document.documentElement.dataset.theme='dark');button('浅色',()=>document.documentElement.dataset.theme='light');button('English',()=>setUiLocale('en'));button('中文',()=>setUiLocale('zh'))
button('Xbox',()=>pressed([16]));button('B',()=>pressed([1]));button('背键1',()=>pressed([17]));button('释放',()=>pressed([]));button('组合',()=>pressed([8,9]));button('隐藏工具',()=>toolbar.style.display='none')
button('布局检查',()=>{const result=geometry();document.getElementById('layout-results')?.remove();const node=document.createElement('pre');node.id='layout-results';node.hidden=true;node.textContent=JSON.stringify(result);document.body.append(node)})
const status=document.createElement('span');status.dataset.result='';status.textContent='等待验证';toolbar.append(status)
