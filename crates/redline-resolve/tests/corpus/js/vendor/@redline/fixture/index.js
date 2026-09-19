'use strict';

/**
 * @redline/fixture — a scoped local package. The scoped name
 * (@redline/fixture) plus its ./util subpath exercise the provider's
 * base-package normalization (the first `.`/`::` is the package boundary;
 * `/` stays inside the name).
 */

function banner(text) {
  return `*** ${text} ***`;
}

module.exports = { banner };
