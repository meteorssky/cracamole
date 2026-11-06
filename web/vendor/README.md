# Apache Guacamole JavaScript client

`guacamole.js` concatenates the unmodified `guacamole-common-js` 1.6.0 modules,
with Namespace.js first. This is JavaScript executed by the browser. No Java,
JRE, Maven, Java servlet, or official Java web application is used to build or run
cracamole-web.

Regenerate from the repository root with `python scripts/vendor_guacamole.py`.
The script pins and checks the upstream archive's SHA-256, downloads only to
`target/`, and preserves LICENSE and NOTICE. Source:
https://github.com/apache/guacamole-client/tree/1.6.0/guacamole-common-js
