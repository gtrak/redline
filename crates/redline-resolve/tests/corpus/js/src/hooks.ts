// hooks.ts — the corpus's TypeScript module. It exercises the TS import
// forms over the same corpus packages: a named value import, a type-only
// import, a scoped default import, a scoped SUBPATH import, and a default
// import of a CJS package.

import { greet, add } from 'jslocal';
import type { GreetResult } from 'jslocal';
import { banner } from '@redline/fixture';
import { normalize } from '@redline/fixture/util';
import leftPad from 'left-pad';

interface Row {
  label: string;
  value: number;
}

export function titleOf(name: string): GreetResult {
  return greet(name);
}

export function total(row: Row): number {
  return add(row.value, 1);
}

export function header(row: Row): string {
  return banner(normalize(row.label));
}

export function padded(label: string, width: number): string {
  return leftPad(label, width, '-');
}

export function rows(count: number): Row[] {
  return Array.from({ length: count }, (_, i) => ({
    label: `row-${i}`,
    value: i * 2,
  }));
}
