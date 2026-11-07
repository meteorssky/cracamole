export const DEFAULT_PORTS = { ssh: '22', telnet: '23', rdp: '3389', vnc: '5900' };
export const PROTOCOL_LABELS = { ssh: 'SSH', telnet: 'Telnet', rdp: 'RDP', vnc: 'VNC' };

export function protocolLabel(protocol) {
  return PROTOCOL_LABELS[protocol] || String(protocol).toUpperCase();
}

export function endpoint(connection) {
  const params = connection.params || {};
  const user = params.username ? params.username + '@' : '';
  const port = params.port || DEFAULT_PORTS[connection.protocol] || '';
  return user + params.hostname + (port ? ':' + port : '');
}
