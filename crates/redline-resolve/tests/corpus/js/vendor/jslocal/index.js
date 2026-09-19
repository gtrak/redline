'use strict';

/**
 * jslocal — the corpus's local (file:) package. A tiny greeter with the
 * export shapes a real small CJS package has: a function, an arrow
 * const, and a class, re-exported through one module.exports object.
 */

function greet(name) {
  return `Hello, ${name}!`;
}

const add = (a, b) => a + b;

class Counter {
  constructor(start = 0) {
    this.value = start;
  }

  bump(step = 1) {
    this.value += step;
    return this.value;
  }
}

module.exports = { greet, add, Counter };
