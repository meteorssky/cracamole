'use strict';
const $ = id => document.getElementById(id);
let connections = [], editing = null, client = null, activeId = null, resize = null;
const keyboard = new Guacamole.Keyboard($('terminal'));
const placeholder = $('terminal-placeholder');
keyboard.onkeydown=keysym=>{if(client){client.sendKeyEvent(1,keysym);return false;}return true;};
keyboard.onkeyup=keysym=>client?.sendKeyEvent(0,keysym);
function toast(text) { $('toast').textContent=text; $('toast').hidden=false; clearTimeout(toast.timer); toast.timer=setTimeout(()=>$('toast').hidden=true,3500); }
async function api(path, options={}) {
  const response=await fetch('/api'+path,{...options,headers:{'Content-Type':'application/json',...options.headers},credentials:'same-origin'});
  if (!response.ok) {
    let error; try {error=(await response.json()).error;} catch {error='操作失敗';}
    if (response.status===401 && path!=='/login') showLogin();
    throw new Error(error);
  }
  return response.status===204 ? null : response.json();
}
function disconnect() {
  if (resize) {resize.disconnect();resize=null;}
  keyboard.reset();
  if (client) {client.disconnect();client=null;}
  activeId=null;
  $('disconnect').disabled=true; $('toggle-clipboard').disabled=true;
  $('clipboard-panel').hidden=true; $('clipboard-text').value='';
  $('terminal').replaceChildren(placeholder);$('terminal-name').textContent='準備連線';
  $('terminal-status').textContent='未連線';
  renderConnections();
}
function showLogin() {
  disconnect(); $('workspace').hidden=true; $('login-view').hidden=false;
  if ($('connection-editor').open) $('connection-editor').close();
  $('connection-form').reset();editing=null;connections=[];renderConnections();
}
async function showWorkspace() {
  $('login-view').hidden=true; $('workspace').hidden=false;
  connections=await api('/connections'); renderConnections();
}
function renderConnections() {
  $('connections-list').replaceChildren(); $('empty-state').hidden=connections.length>0;
  for (const connection of connections) {
    const card=document.createElement('article'); card.className='connection-card'+(connection.id===activeId?' active':'');
    const open=document.createElement('button'); open.className='connection-open'; open.textContent=connection.name; open.setAttribute('aria-label','連線 '+connection.name);
    open.onclick=()=>connect(connection);
    const meta=document.createElement('p');meta.className='connection-meta';meta.textContent=connection.protocol.toUpperCase()+' · '+connection.params.hostname+':'+(connection.params.port||(connection.protocol==='ssh'?'22':'23'));
    const actions=document.createElement('div');actions.className='connection-actions';
    const edit=document.createElement('button');edit.textContent='編輯';edit.setAttribute('aria-label','編輯 '+connection.name);edit.onclick=()=>editConnection(connection);
    const remove=document.createElement('button');remove.textContent='刪除';remove.setAttribute('aria-label','刪除 '+connection.name);
    remove.onclick=async()=>{if(!confirm('確定刪除「'+connection.name+'」？'))return;try{await api('/connections/'+connection.id,{method:'DELETE'});if(activeId===connection.id)disconnect();await showWorkspace();}catch(e){toast(e.message);}};
    actions.append(edit,remove);card.append(open,meta,actions);$('connections-list').append(card);
  }
}
const DEFAULT_PORTS={ssh:'22',telnet:'23',rdp:'3389'};
function editConnection(connection=null) {
  editing=connection;const form=$('connection-form');form.reset();$('editor-error').textContent='';
  $('editor-title').textContent=connection?'編輯連線':'新增連線';
  for(const name of ['name','protocol']) if(connection) form.elements.namedItem(name).value=connection[name];
  const params=connection?.params||{};
  for(const name of ['hostname','port','username','host-key','login-success-regex','login-failure-regex','domain']) form.elements.namedItem(name).value=params[name]||'';
  form.elements.security.value=params.security||'any';form.elements['ignore-cert'].checked=params['ignore-cert']==='true';
  for(const secret of ['password','private-key','passphrase']) form.elements.namedItem(secret).placeholder=connection?.configured_secrets.includes(secret)?'已設定；留空保留':'';
  form.elements.port.placeholder=DEFAULT_PORTS[form.elements.protocol.value];
  $('connection-editor').showModal();form.elements.name.focus();
}
function paste(text) {
  if(!client)return;
  const writer=new Guacamole.StringWriter(client.createClipboardStream('text/plain'));writer.sendText(text);writer.sendEnd();
}
function connect(connection) {
  disconnect(); activeId=connection.id;renderConnections();
  $('terminal-name').textContent=connection.name;$('terminal-status').textContent='正在連線…';
  const terminal=$('terminal');terminal.replaceChildren();
  const scheme=location.protocol==='https:'?'wss:':'ws:';
  const tunnel=new Guacamole.WebSocketTunnel(scheme+'//'+location.host+'/api/connections/'+connection.id+'/tunnel');
  const current=new Guacamole.Client(tunnel);client=current;
  let failure=null;
  const display=current.getDisplay();terminal.append(display.getElement());
  current.onerror=tunnel.onerror=error=>{
    if(client!==current)return;
    failure=error.message||'連線失敗';current.disconnect();
  };
  tunnel.onstatechange=state=>{
    if(client===current && state===Guacamole.Tunnel.State.CLOSED)current.disconnect();
  };
  current.onstatechange=state=>{
    if(client!==current)return;
    if(state===3){$('terminal-status').textContent='已連線';terminal.focus();}
    if(state===5){
      $('terminal-status').textContent=failure||'連線已結束';
      $('disconnect').disabled=true;$('toggle-clipboard').disabled=true;
      resize?.disconnect();keyboard.reset();
    }
  };
  current.onclipboard=(stream,mimetype)=>{
    if(!mimetype.startsWith('text/'))return;
    const reader=new Guacamole.StringReader(stream);let text='';
    reader.ontext=part=>{if(text.length<262144)text+=part;};reader.onend=()=>{$('clipboard-text').value=text;};
  };
  const mouse=new Guacamole.Mouse(display.getElement());
  mouse.onmousedown=mouse.onmouseup=mouse.onmousemove=state=>{terminal.focus();current.sendMouseState(state,true);};
  terminal.onblur=()=>keyboard.reset();
  resize=new ResizeObserver(()=>{if(client===current)current.sendSize(Math.max(1,terminal.clientWidth),Math.max(1,terminal.clientHeight));});
  resize.observe(terminal);
  current.connect(new URLSearchParams({width:terminal.clientWidth,height:terminal.clientHeight,dpi:96}).toString());
  $('disconnect').disabled=false;$('toggle-clipboard').disabled=false;
}
$('terminal').addEventListener('keydown',event=>{if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='v')event.stopImmediatePropagation();},true);
$('terminal').addEventListener('paste',event=>{event.preventDefault();paste(event.clipboardData.getData('text/plain'));});
$('login-form').addEventListener('submit',async event=>{
  event.preventDefault();const form=event.target,button=form.querySelector('button');button.disabled=true;$('login-error').textContent='';
  try{await api('/login',{method:'POST',body:JSON.stringify({username:form.elements.username.value,password:form.elements.password.value})});form.reset();await showWorkspace();}
  catch(error){$('login-error').textContent=error.message;}finally{button.disabled=false;}
});
$('connection-form').addEventListener('submit',async event=>{
  event.preventDefault();const form=event.target,params={...(editing?.params||{})},protocol=form.elements.protocol.value;
  for(const key of ['hostname','port','username']) params[key]=form.elements.namedItem(key).value.trim();
  for(const key of ['password','private-key','passphrase']) {
    const value=form.elements.namedItem(key).value;
    if(value||form.elements.namedItem('clear-secrets').checked)params[key]=value;
  }
  if(protocol==='rdp'){
    for(const key of ['terminal-type','func-keys-and-keypad','host-key','private-key','passphrase','login-success-regex','login-failure-regex'])delete params[key];
    params.domain=form.elements.domain.value.trim();params.security=form.elements.security.value;params['ignore-cert']=form.elements['ignore-cert'].checked?'true':'false';
  } else {
  for(const key of ['domain','security','ignore-cert'])delete params[key];
  params['terminal-type']='xterm';params['func-keys-and-keypad']='vt100';
  if(protocol==='ssh'){params['host-key']=form.elements.namedItem('host-key').value.trim();delete params['login-success-regex'];delete params['login-failure-regex'];}
  else {params['login-success-regex']=form.elements.namedItem('login-success-regex').value;params['login-failure-regex']=form.elements.namedItem('login-failure-regex').value;delete params['host-key'];delete params['private-key'];delete params.passphrase;}
  }
  try {await api('/connections'+(editing?'/'+editing.id:''),{method:editing?'PUT':'POST',body:JSON.stringify({name:form.elements.name.value,protocol,params})});$('connection-editor').close();form.reset();await showWorkspace();}
  catch(error){$('editor-error').textContent=error.message;}
});
$('connection-form').elements.protocol.onchange=event=>{$('connection-form').elements.port.placeholder=DEFAULT_PORTS[event.target.value];};
$('add-connection').onclick=()=>editConnection();
$('close-editor').onclick=$('cancel-editor').onclick=()=>{$('connection-editor').close();$('connection-form').reset();};
$('disconnect').onclick=disconnect;
$('toggle-clipboard').onclick=()=>{$('clipboard-panel').hidden=!$('clipboard-panel').hidden;};
$('paste-clipboard').onclick=()=>{paste($('clipboard-text').value);$('terminal').focus();};
$('copy-clipboard').onclick=async()=>{try{await navigator.clipboard.writeText($('clipboard-text').value);toast('已複製');}catch{$('clipboard-text').select();toast('請手動複製所選內容');}};
$('logout').onclick=async()=>{try{await api('/logout',{method:'POST'});showLogin();}catch(error){toast(error.message);}};
window.addEventListener('beforeunload',()=>client?.disconnect());
api('/session').then(showWorkspace).catch(()=>showLogin());
