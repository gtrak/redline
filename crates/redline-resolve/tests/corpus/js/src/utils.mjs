// utils.mjs — the corpus's ESM module (.mjs is unambiguous under any
// package.json "type"). It uses the ESM import forms: named, default,
// namespace — plus a side-effect stylesheet import that binds nothing.

import * as local from 'jslocal';
import { getProperty } from 'dot-prop';
import defaultPad from 'left-pad';
import './styles.css';

export function padRow(label, width) {
  return defaultPad(label, width, ' ');
}

export function hello(name) {
  return local.greet(name);
}

export function readConfig(config, path) {
  return getProperty(config, path);
}

// Re-export the namespace object itself: a jump target for "which module
// is this?"-style navigation (bare `local`, 1-segment hint).
export { local as localApi };
