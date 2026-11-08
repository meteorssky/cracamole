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
