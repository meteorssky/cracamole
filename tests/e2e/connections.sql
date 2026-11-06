INSERT INTO guacamole_connection (connection_name, protocol)
VALUES ('SSH MVP', 'ssh'), ('Telnet MVP', 'telnet');

INSERT INTO guacamole_connection_parameter (connection_id, parameter_name, parameter_value)
SELECT connection_id, 'hostname', 'backend' FROM guacamole_connection;
INSERT INTO guacamole_connection_parameter (connection_id, parameter_name, parameter_value)
SELECT connection_id, 'username', 'tester' FROM guacamole_connection;
INSERT INTO guacamole_connection_parameter (connection_id, parameter_name, parameter_value)
SELECT connection_id, 'password', 'tester-pw' FROM guacamole_connection;
INSERT INTO guacamole_connection_parameter (connection_id, parameter_name, parameter_value)
SELECT connection_id, 'terminal-type', 'xterm' FROM guacamole_connection;
