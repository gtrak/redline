'use strict';

// The subpath module: `@redline/fixture/util` resolves INSIDE this
// package (the base package keeps its `@scope/name` prefix).

function normalize(input) {
  return String(input).trim().toLowerCase();
}

module.exports = { normalize };
