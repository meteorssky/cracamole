// The only module that touches the Guacamole client library. The rest of the app sees a Session.
import { MODIFIER_KEYSYMS } from './keys.js';
import { classify } from './reasons.js';
import { encodeClipboard, receiveClipboard, sendClipboard } from './text.js';

const keyboards = new WeakMap(); // Guacamole.Keyboard cannot be detached, so one per mount element
let libraryLoading = null;

// Share an in-flight load, but let a failed request be retried on the next connection.
export function loadGuacamole() {
  if (globalThis.Guacamole?.Client) return Promise.resolve();
  if (libraryLoading) return libraryLoading;
  libraryLoading = new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = new URL('../../vendor/guacamole.js', import.meta.url).href;
    script.onload = () => {
      if (globalThis.Guacamole?.Client) resolve();
      else {
        script.remove();
        reject(new Error('Guacamole client unavailable'));
      }
    };
    script.onerror = () => {
      script.remove();
      reject(new Error('Guacamole client failed to load'));
    };
    document.head.append(script);
  }).catch(error => {
    libraryLoading = null;
    throw error;
  });
  return libraryLoading;
}

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
export function createSession({ connection, mount, onState, onClipboard, onModifiers, onEscape,
  onClipboardError = () => {} }) {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const tunnel = new Guacamole.WebSocketTunnel(
    scheme + '//' + location.host + '/api/connections/' + encodeURIComponent(connection.id) + '/tunnel',
  );
  const client = new Guacamole.Client(tunnel);
  const display = client.getDisplay();
  const host = document.createElement('div');
  host.append(display.getElement());
  mount.append(host);
  const mobileInput = document.createElement('textarea');
  mobileInput.className = 'terminal-ime';
  mobileInput.setAttribute('aria-label', '遠端文字輸入');
  mobileInput.setAttribute('autocomplete', 'off');
  mobileInput.setAttribute('autocapitalize', 'off');
  mobileInput.setAttribute('spellcheck', 'false');
  mobileInput.maxLength = 4096;
  mobileInput.tabIndex = -1;
  mount.append(mobileInput);

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

  function sendMobileKey(keysym) {
    if (disposed || state !== 'connected') return;
    pressModifiers();
    key(true, keysym);
    key(false, keysym);
    releaseModifiers();
  }

  function flushMobileText() {
    const text = mobileInput.value;
    mobileInput.value = '';
    for (const character of text) {
      const code = character.codePointAt(0);
      if (code === 10 || code === 13) sendMobileKey(0xff0d);
      else if (code >= 0x20 && code <= 0x10ffff && code !== 0x7f) {
        sendMobileKey(code <= 0xff ? code : 0x01000000 | code);
      }
    }
  }

  let composing = false;
  const onCompositionStart = event => { event.stopPropagation(); composing = true; };
  const onCompositionEnd = event => {
    event.stopPropagation();
    composing = false;
    queueMicrotask(() => { if (!disposed && !composing) flushMobileText(); });
  };
  const onMobileInput = event => {
    event.stopPropagation();
    if (event.isComposing || composing) return;
    if (event.inputType === 'deleteContentBackward' && !mobileInput.value) sendMobileKey(0xff08);
    else flushMobileText();
  };
  const onMobileBeforeInput = event => {
    if (event.isComposing || composing) return;
    if (event.inputType === 'deleteContentBackward' || event.inputType === 'insertLineBreak') {
      event.preventDefault();
      sendMobileKey(event.inputType === 'deleteContentBackward' ? 0xff08 : 0xff0d);
    }
  };
  const onMobileKeyDown = event => {
    event.stopPropagation();
    if (event.isComposing || composing) return;
    const special = { Backspace: 0xff08, Enter: 0xff0d, Tab: 0xff09, Escape: 0xff1b,
      ArrowLeft: 0xff51, ArrowUp: 0xff52, ArrowRight: 0xff53, ArrowDown: 0xff54 };
    if (Object.hasOwn(special, event.key)) {
      event.preventDefault();
      sendMobileKey(special[event.key]);
    }
  };
  const onTouchFocus = event => {
    if (event.pointerType === 'touch' && state === 'connected') mobileInput.focus();
  };
  const onMobileKeyUp = event => event.stopPropagation();
  mobileInput.addEventListener('compositionstart', onCompositionStart);
  mobileInput.addEventListener('compositionend', onCompositionEnd);
  mobileInput.addEventListener('input', onMobileInput);
  mobileInput.addEventListener('beforeinput', onMobileBeforeInput);
  mobileInput.addEventListener('keydown', onMobileKeyDown);
  mobileInput.addEventListener('keyup', onMobileKeyUp);
  mount.addEventListener('pointerdown', onTouchFocus);

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
    receiveClipboard(stream, onClipboard, onClipboardError);
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
  // Ctrl+Shift+Escape is the keyboard escape hatch out of the terminal focus trap.
  const onKeyDown = event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v') {
      event.stopImmediatePropagation();
      return;
    }
    if (event.ctrlKey && event.shiftKey && event.key === 'Escape') {
      event.stopImmediatePropagation();
      event.preventDefault();
      onEscape?.();
    }
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
      if (state !== 'connected' || !text) return false;
      try {
        const bytes = encodeClipboard(text);
        sendClipboard(client, bytes);
        return true;
      } catch (error) {
        onClipboardError(error.message);
        return false;
      }
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
      mount.removeEventListener('pointerdown', onTouchFocus);
      mobileInput.removeEventListener('compositionstart', onCompositionStart);
      mobileInput.removeEventListener('compositionend', onCompositionEnd);
      mobileInput.removeEventListener('input', onMobileInput);
      mobileInput.removeEventListener('beforeinput', onMobileBeforeInput);
      mobileInput.removeEventListener('keydown', onMobileKeyDown);
      mobileInput.removeEventListener('keyup', onMobileKeyUp);
      mobileInput.remove();
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
