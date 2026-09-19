// app.js — the corpus entry point. CommonJS on purpose: this file carries
// the require(...) shapes the app's tree-sitter hand-off passes to the JS
// provider: whole-module bindings, destructured named bindings, dotted
// call sites, and a relative sibling load.

'use strict';

const leftPad = require('left-pad');
const pascalCase = require('pascal-case');
const { greet, add } = require('jslocal');
const local = require('jslocal');
const { banner } = require('@redline/fixture');
const legacy = require('./legacy-util');

function padColumn(label, width) {
  return leftPad(label, width, '.');
}

function titleCase(label) {
  return pascalCase(label);
}

function bannerLine(title) {
  return banner(title.toUpperCase());
}

function summary(n) {
  const name = greet(`row ${n}`);
  const total = add(n, 1);
  return [name, total, local.add(n, 2)].join(' ');
}

function legacyRow(parts) {
  return legacy.legacyJoin(parts);
}

// Probe anchors (011-08, js lane): the golden probes point at the input
// shapes a jump would carry — the symbol plus the import-hint (scope) the
// app's tree-sitter hand-off supplies. The lines below document shapes
// that have no honest live use site in this file.
// [probe:local-item-missing-bail]   local.missing()    hint: ["jslocal", "missing"]
// [probe:absolute-import-bail]      legacyJoin         hint: ["/abs/legacy-util", "legacyJoin"]
// [probe:no-package-json-bail]      acme.doThing, probed in a directory with
//                                    no package.json anywhere up the tree
// [probe:live-dotprop-item-missing] the v6 `get` name against dot-prop v7,
//                                    hint: ["dot-prop", "get"]
// [probe:relative-member-live-ws]   the relative hint from legacy.cjs, probed
//                                    where node_modules IS populated

function main() {
  for (let i = 1; i <= 3; i += 1) {
    console.log(padColumn(titleCase(summary(i)), 16));
  }
  console.log(bannerLine('corpus'));
  console.log(legacyRow(['a', 'b']));
}

if (require.main === module) {
  main();
}

module.exports = { padColumn, titleCase, bannerLine, summary, legacyRow };
