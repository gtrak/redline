// legacy-util.js — a sibling CommonJS module, loaded relatively from
// legacy.cjs. The specifier is workspace-local (`./legacy-util`): the JS
// provider must never claim it, and the js lane's relative/absolute
// import bails are anchored at this file's two load sites.

'use strict';

function legacyJoin(parts) {
  return parts.join(' | ');
}

function legacyPad(text) {
  return `:: ${text} ::`;
}

module.exports = { legacyJoin, legacyPad };
