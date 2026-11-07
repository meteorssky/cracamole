// The only place that calls fetch. Normalises failures to ApiError { status, message, field }.
const TIMEOUT_MS = 15000;
let onUnauthorized = () => {};

export class ApiError extends Error {
  constructor({ status, message, field = null }) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.field = field;
  }
}

export function setUnauthorizedHandler(handler) {
  onUnauthorized = handler;
}

export async function api(path, { method = 'GET', body } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  let response;
  try {
    response = await fetch('/api' + path, {
      method,
      credentials: 'same-origin',
      signal: controller.signal,
      headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError({
      status: 0,
      message: '無法連到 cracamole 伺服器。請檢查網路連線，稍後再試一次。',
    });
  } finally {
    clearTimeout(timer);
  }
  if (!response.ok) {
    let payload = {};
    try {
      payload = await response.json();
    } catch {
      payload = {};
    }
    const error = new ApiError({
      status: response.status,
      message: payload.error || '操作失敗，請稍後再試一次。',
      field: payload.field || null,
    });
    if (response.status === 401 && path !== '/login') onUnauthorized(error);
    throw error;
  }
  return response.status === 204 ? null : response.json();
}
