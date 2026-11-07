// One small observable store. Views read and write through it instead of touching each other.
const data = { loggedIn: false, connections: [], activeId: null };
const listeners = new Map();

export const state = {
  get(key) {
    return data[key];
  },
  set(key, value) {
    data[key] = value;
    for (const listener of listeners.get(key) || []) listener(value);
  },
  on(key, listener) {
    if (!listeners.has(key)) listeners.set(key, new Set());
    listeners.get(key).add(listener);
  },
};
