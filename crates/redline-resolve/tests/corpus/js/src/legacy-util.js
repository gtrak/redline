// legacy-util.js — a sibling CommonJS module, loaded relatively from
// legacy.cjs and app.js. The specifier is workspace-local (`./legacy-util`):
// the JS provider RESOLVES it against the importing buffer's directory and
// lands in this file (external = false) — the js lane's relative/absolute
// import probes are anchored at these load sites (fix-jsrel; in 011-08 the
// shape used to mangle into the base-package-`.` degradation).

'use strict';

function legacyJoin(parts) {
  return parts.join(' | ');
}

function legacyPad(text) {
  return `:: ${text} ::`;
}

module.exports = { legacyJoin, legacyPad };
