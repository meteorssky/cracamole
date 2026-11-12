// Protocol catalogue, loaded once from the server (GET /api/protocols) after login.
// The server is the only place that knows which protocols exist, their label, default port,
// accepted parameters and which server identity (host key / certificate) can be pinned.
import { api } from '../api.js';

let catalogue = [];

export async function loadProtocols() {
  catalogue = await api('/protocols');
  return catalogue;
}

export const allProtocols = () => catalogue;

// Parse only connection addresses, never a URI containing a password, path or query.
export function quickAddress(value, defaultProtocol = 'ssh') {
  let text = value.trim();
  if (!text || text.length > 320) return null;
  // This is an address convenience, not a shell parser: accept only the explicit
  // `ssh -p PORT [USER@]HOST` shape, with no quoting, extra options or command.
  if (/^ssh\s/i.test(text)) {
    const command = text.match(/^ssh\s+-p\s+([0-9]{1,5})\s+([^\s]+)$/i);
    if (!command || /^-/.test(command[2]) || /['"`$;&|\\]/.test(command[2]) ||
        /:/.test(command[2].replace(/\[[0-9a-fA-F:.]+\]/, ''))) return null;
    text = 'ssh://' + command[2] + ':' + command[1];
  }
  const match = text.match(/^(?:([a-z][a-z0-9+.-]*):\/\/)?(?:([^@:\s/?#]+)@)?(\[[0-9a-fA-F:.]+\]|[^:\s/?#@]+)(?::([0-9]{1,5}))?$/i);
  if (!match) return null;
  const protocol = (match[1] || defaultProtocol).toLowerCase();
  if (!find(protocol) || (match[4] && (Number(match[4]) < 1 || Number(match[4]) > 65535))) return null;
  const hostname = match[3].startsWith('[') ? match[3].slice(1, -1) : match[3];
  if (!hostname || hostname.length > 255 || hostname.includes('\\')) return null;
  let username = match[2] || '';
  if (match[1] && username.includes('%')) {
    try {
      username = decodeURIComponent(username);
    } catch {
      return null;
    }
  }
  if (username.includes(':') || /[\u0000-\u001f\u007f]/.test(username)) return null;
  let domain = '';
  if (protocol === 'rdp' && username.includes('\\')) {
    const parts = username.split('\\');
    if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
    [domain, username] = parts;
  }
  return {
    protocol, username, domain, hostname, port: match[4] || '',
    explicit: Boolean(match[1] || match[2] || match[4]),
    protocolExplicit: Boolean(match[1]),
  };
}

const find = name => catalogue.find(protocol => protocol.name === name);

export function protocolLabel(protocol) {
  return find(protocol)?.label || String(protocol).toUpperCase();
}

export function defaultPort(protocol) {
  const port = find(protocol)?.default_port;
  return port ? String(port) : '';
}

/** Parameter names the protocol accepts; the server rejects any others. */
export function protocolArgs(protocol) {
  return new Set(find(protocol)?.args || []);
}

/** 'host-key', 'certificate' or null when the protocol has no server identity to pin. */
export function identityKind(protocol) {
  return find(protocol)?.identity || null;
}

export function endpoint(connection) {
  const params = connection.params || {};
  const user = params.username ? params.username + '@' : '';
  const port = params.port || defaultPort(connection.protocol);
  return user + params.hostname + (port ? ':' + port : '');
}
