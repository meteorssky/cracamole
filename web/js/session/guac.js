// The only module that touches the Guacamole client library. The rest of the app sees a Session.
import { MODIFIER_KEYSYMS } from './keys.js';
import { classify } from './reasons.js';

const CLIPBOARD_LIMIT = 262144;
const keyboards = new WeakMap(); // Guacamole.Keyboard cannot be detached, so one per mount element

function keyboardFor(element) {
  let keyboard = keyboards.get(element);
  if (!keyboard) {
    keyboard = new Guacamole.Keyboard(element);
    keyboards.set(element, keyboard);
  }
  return keyboard;
}

/**
 * @param {object} options
 * @param {{id:string, protocol:string}} options.connection
 * @param {HTMLElement} options.mount  element that receives the remote display and keyboard focus
 * @param {(state:'connecting'|'connected'|'closed', info?:object) => void} options.onState
 * @param {(text:string) => void} options.onClipboard
 * @param {(modifiers:{ctrl:boolean, alt:boolean}) => void} options.onModifiers
 */
export function createSession({ connection, mount, onState, onClipboard, onModifiers }) {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const tunnel = new Guacamole.WebSocketTunnel(
    scheme + '//' + location.host + '/api/connections/' + encodeURIComponent(connection.id) + '/tunnel',
  );
  const client = new Guacamole.Client(tunnel);
  const display = client.getDisplay();
  const host = document.createElement('div');
  host.append(display.getElement());
  mount.append(host);

  const modifiers = { ctrl: false, alt: false };
  let state = 'connecting';
  let failure = null;
  let userInitiated = false;
  let wasConnected = false;
  let disposed = false;

  function setState(next, info) {
    if (disposed || state === 'closed') return;
    state = next;
    onState(next, info);
  }

  function key(pressed, keysym) {
    client.sendKeyEvent(pressed ? 1 : 0, keysym);
  }

  function releaseModifiers() {
    for (const name of ['alt', 'ctrl']) {
      if (modifiers[name]) {
        key(false, MODIFIER_KEYSYMS[name]);
        modifiers[name] = false;
      }
    }
    onModifiers({ ...modifiers });
  }

  // Sticky modifiers from the helper bar apply to exactly one following key.
  function pressModifiers() {
    for (const name of ['ctrl', 'alt']) if (modifiers[name]) key(true, MODIFIER_KEYSYMS[name]);
  }

  const keyboard = keyboardFor(mount);
  keyboard.onkeydown = keysym => {
    if (disposed) return true;
    const sticky = modifiers.ctrl || modifiers.alt;
    if (sticky) pressModifiers();
    key(true, keysym);
    if (sticky) {
      key(false, keysym);
      releaseModifiers();
    }
    return false;
  };
  keyboard.onkeyup = keysym => {
    if (disposed) return;
    key(false, keysym);
  };

  function fail(error) {
    if (disposed || failure) return;
    failure = { code: Number(error?.code) || 0, message: error?.message || '' };
    client.disconnect();
  }
  client.onerror = fail;
  tunnel.onerror = fail;
  tunnel.onstatechange = tunnelState => {
    if (tunnelState === Guacamole.Tunnel.State.CLOSED) client.disconnect();
  };
  client.onstatechange = clientState => {
    if (clientState === Guacamole.Client.State.CONNECTED) {
      wasConnected = true;
      setState('connected');
    } else if (clientState === Guacamole.Client.State.DISCONNECTED) {
      setState('closed', classify({ wasConnected, userInitiated, failure }));
    }
  };
  client.onclipboard = (stream, mimetype) => {
    if (!mimetype.startsWith('text/')) return;
    const reader = new Guacamole.StringReader(stream);
    let text = '';
    reader.ontext = part => {
      if (text.length < CLIPBOARD_LIMIT) text += part;
    };
    reader.onend = () => onClipboard(text);
  };

  const mouse = new Guacamole.Mouse(display.getElement());
  const forwardMouse = mouseState => {
    mount.focus();
    client.sendMouseState(mouseState, true);
  };
  mouse.onmousedown = mouse.onmouseup = mouse.onmousemove = forwardMouse;

  const resizeObserver = new ResizeObserver(() => {
    if (state === 'connected') client.sendSize(Math.max(1, mount.clientWidth), Math.max(1, mount.clientHeight));
  });
  resizeObserver.observe(mount);
  const onBlur = () => keyboard.reset();
  mount.addEventListener('blur', onBlur);

  // Ctrl+V is handled by the paste event so the remote receives the text, not a keystroke.
  const onKeyDown = event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v') event.stopImmediatePropagation();
  };
  const onPaste = event => {
    event.preventDefault();
    session.paste(event.clipboardData.getData('text/plain'));
  };
  mount.addEventListener('keydown', onKeyDown, true);
  mount.addEventListener('paste', onPaste);

  const session = {
    connection,
    get state() {
      return state;
    },
    paste(text) {
      if (state !== 'connected' || !text) return;
      const writer = new Guacamole.StringWriter(client.createClipboardStream('text/plain'));
      writer.sendText(text);
      writer.sendEnd();
    },
    sendKey(keysym) {
      if (state !== 'connected') return;
      pressModifiers();
      key(true, keysym);
      key(false, keysym);
      releaseModifiers();
    },
    sendCombo(keysyms) {
      if (state !== 'connected') return;
      for (const keysym of keysyms) key(true, keysym);
      for (const keysym of [...keysyms].reverse()) key(false, keysym);
    },
    toggleModifier(name) {
      modifiers[name] = !modifiers[name];
      onModifiers({ ...modifiers });
    },
    focus() {
      mount.focus();
    },
    disconnect() {
      userInitiated = true;
      client.disconnect();
    },
    dispose() {
      if (disposed) return;
      userInitiated = true;
      disposed = true;
      resizeObserver.disconnect();
      mount.removeEventListener('blur', onBlur);
      mount.removeEventListener('keydown', onKeyDown, true);
      mount.removeEventListener('paste', onPaste);
      keyboard.onkeydown = null;
      keyboard.onkeyup = null;
      keyboard.reset();
      client.disconnect();
      host.remove();
    },
  };

  client.connect(
    new URLSearchParams({ width: mount.clientWidth, height: mount.clientHeight, dpi: 96 }).toString(),
  );
  onState('connecting');
  return session;
}
