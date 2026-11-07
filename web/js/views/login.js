import { api, ApiError } from '../api.js';
import { $ } from '../util/dom.js';

export function showLogin(notice = '') {
  $('workspace').hidden = true;
  $('login-view').hidden = false;
  const note = $('login-notice');
  note.textContent = notice;
  note.hidden = !notice;
  $('login-error').hidden = true;
  $('login-form').reset();
  $('login-password').type = 'password';
  $('login-username').focus();
}

export function hideLogin() {
  $('login-view').hidden = true;
}

export function initLogin({ onLoggedIn }) {
  $('login-reveal').addEventListener('change', event => {
    $('login-password').type = event.target.checked ? 'text' : 'password';
  });
  $('login-form').addEventListener('submit', async event => {
    event.preventDefault();
    const form = event.target;
    const button = form.querySelector('button[type="submit"]');
    const error = $('login-error');
    error.hidden = true;
    if (!form.elements.username.value || !form.elements.password.value) {
      error.textContent = '請輸入管理員帳號與密碼。';
      error.hidden = false;
      return;
    }
    button.disabled = true;
    button.setAttribute('aria-busy', 'true');
    try {
      await api('/login', {
        method: 'POST',
        body: { username: form.elements.username.value, password: form.elements.password.value },
      });
      form.reset();
      await onLoggedIn();
    } catch (failure) {
      error.textContent = failure instanceof ApiError ? failure.message : '登入失敗，請再試一次。';
      error.hidden = false;
      $('login-password').select();
    } finally {
      button.disabled = false;
      button.removeAttribute('aria-busy');
    }
  });
}
