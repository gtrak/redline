// legacy.cjs — a CommonJS-only module (the .cjs extension pins the format
// in the corpus's mixed module system). It loads the sibling module both
// ways: a whole-module binding and a destructured named binding.

'use strict';

const legacy = require('./legacy-util');
const { legacyJoin } = require('./legacy-util');

function joinAll(rows) {
  return legacyJoin(rows);
}

function paddedRow(text) {
  return legacy.legacyPad(text);
}

module.exports = { joinAll, paddedRow };
