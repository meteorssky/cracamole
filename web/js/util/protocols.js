export const DEFAULT_PORTS = { ssh: '22', telnet: '23', rdp: '3389' };
export const PROTOCOL_LABELS = { ssh: 'SSH', telnet: 'Telnet', rdp: 'RDP' };

export function protocolLabel(protocol) {
  return PROTOCOL_LABELS[protocol] || String(protocol).toUpperCase();
}

export function endpoint(connection) {
  const params = connection.params || {};
  const port = params.port || DEFAULT_PORTS[connection.protocol] || '';
  return params.hostname + (port ? ':' + port : '');
}
