import { $ } from '../util/dom.js';
import { toast } from '../ui/toast.js';

// The connection library: always visible when idle, tucked away during a session to give the terminal the space.
export function setDrawerOpen(open) {
  $('library').hidden = !open;
  const button = $('toggle-library');
  button.setAttribute('aria-expanded', String(open));
  button.setAttribute('aria-label', open ? '收合連線庫' : '開啟連線庫');
}

export function isDrawerOpen() {
  return !$('library').hidden;
}

// During an active session, the drawer floats over the stage so opening it never resizes the remote desktop/PTY.
export function setDrawerOverlay(overlay) {
  const library = $('library');
  if (overlay) library.dataset.overlay = 'true';
  else delete library.dataset.overlay;
}

// Collapses the top bar and status bar so the SSH/RDP stage gets 100% of the viewport height.
export function setChromeCollapsed(collapsed) {
  const workspace = $('workspace');
  if (collapsed) {
    workspace.dataset.chrome = 'collapsed';
    setDrawerOpen(false);
  } else {
    delete workspace.dataset.chrome;
  }
  $('topbar').hidden = collapsed;
  $('statusbar').hidden = collapsed;
  $('chrome-pill').hidden = !collapsed;
  $('collapse-chrome').setAttribute('aria-expanded', String(!collapsed));
}

export function isChromeCollapsed() {
  return $('workspace').dataset.chrome === 'collapsed';
}

export function initDrawer() {
  $('toggle-library').addEventListener('click', () => setDrawerOpen(!isDrawerOpen()));
  $('close-library').addEventListener('click', () => {
    setDrawerOpen(false);
    $('toggle-library').focus();
  });
  document.querySelector('.topbar .brand')?.addEventListener('click', event => {
    event.preventDefault();
    setDrawerOpen(!isDrawerOpen());
  });
  $('library').addEventListener('keydown', event => {
    if (event.key === 'Escape' && $('library').dataset.overlay === 'true') {
      event.stopPropagation();
      setDrawerOpen(false);
      $('toggle-library').focus();
    }
  });
  $('collapse-chrome').addEventListener('click', () => {
    setChromeCollapsed(true);
    $('terminal').focus();
  });
  $('expand-chrome').addEventListener('click', () => {
    setChromeCollapsed(false);
    $('terminal').focus();
  });
  $('toggle-fullscreen').addEventListener('click', async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch {
      toast('瀏覽器不允許切換全螢幕模式。');
    }
  });
  document.addEventListener('fullscreenchange', () => {
    const full = Boolean(document.fullscreenElement);
    $('toggle-fullscreen').setAttribute('aria-pressed', String(full));
    $('toggle-fullscreen').textContent = full ? '離開全螢幕' : '全螢幕';
  });
}
