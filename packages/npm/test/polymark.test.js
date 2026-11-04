import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const consumer = process.env.POLYMARK_PACKAGE;

async function entry() {
  if (!consumer) return new URL('../dist/polymark.js', import.meta.url).href;
  const manifest = resolve(consumer, 'package.json');
  const name = Object.keys(JSON.parse(await readFile(manifest, 'utf8')).dependencies).find((d) => d === '@meritt/polymark-wasm');
  return pathToFileURL(createRequire(manifest).resolve(name)).href;
}

const { PolymarkError, load } = await import(await entry());

const polymark = await load();
const { version } = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));

const corpusPath = process.env.POLYMARK_CORPUS ?? fileURLToPath(new URL('../../../testdata/cases.json', import.meta.url));
const corpus = JSON.parse(await readFile(corpusPath, 'utf8'));

const input = (c) => c.input.repeat(c.repeat ?? 1);

test('corpus format', () => {
  assert.equal(corpus.format, 1);
});

test('transform corpus cases', () => {
  assert.ok(corpus.transform.length > 0);
  for (const c of corpus.transform) {
    assert.deepEqual(polymark.transform(input(c), c.from, c.to), c.want, c.name);
  }
});

test('clean corpus cases', () => {
  assert.ok(corpus.clean.length > 0);
  for (const c of corpus.clean) {
    assert.deepEqual(polymark.clean(input(c)), c.want, c.name);
  }
});

test('reports the version of the package', () => {
  assert.equal(polymark.version, version);
});

test('refuses text that is not well-formed and unknown formats', () => {
  assert.deepEqual(polymark.transform('a\ud800', 'plain', 'html'), { error: 'unicode' });
  assert.throws(
    () => polymark.transform('a', 'telegram', 'html'),
    (error) => error instanceof PolymarkError && error.code === 'invalid_argument'
  );
});

test('refuses a module from another build', async () => {
  await assert.rejects(
    load(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])),
    (error) => error instanceof PolymarkError && error.code === 'version'
  );
});
