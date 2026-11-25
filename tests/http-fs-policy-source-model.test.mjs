// Pure source model, NOT Atto compiler/guest/host filesystem evidence.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const load = path => readFileSync(new URL('../' + path, import.meta.url), 'utf8');
const paths = ['fs_api_policy', 'fs_io_policy', 'fs_text_policy', 'http_response_policy'];
const source = paths.map(name => load('stdlib/std/' + name + '.atto')).join('\n');
const MAX = 9223372036854775807n;
function translate(text) {
  const cleaned = text.replace(/\/\/[^\n]*/g, '');
  if (/\b(import|require|eval|class|actor|while|for|new)\b/.test(cleaned))
    throw new Error('unsupported source model syntax');
  const js = cleaned
    .replace(/\bpub fnc (\w+)\(([^)]*)\): (?:bool|int) \{/g,
      (_, name, params) => 'function ' + name + '(' +
        params.split(',').map(p => p.trim().split(/\s+/).at(-1)).filter(Boolean).join(',') + ') {')
    .replace(/\bif (.*?) then return (.*?); fi/g, (_, cond, result) =>
      'if (' + cond + ') { return ' + result + '; }')
    .replace(/(?<![A-Za-z_])([0-9]+)(?![A-Za-z_])/g, '$1n');
  if (/\b(pub|fnc|then|fi)\b/.test(js)) throw new Error('untranslated syntax');
  return js;
}
function model(text) {
  const scope = Object.create(null);
  runInNewContext(translate(text), scope, {timeout: 2000, codeGeneration: {strings:false,wasm:false}});
  return scope;
}
test('all 63 independent pure FS fixture guards execute in BigInt source model', () => {
  for (const [name, count] of [['fs_api_policy_main',32],['fs_io_policy_main',31]]) {
    const f = load('fixtures/fs/' + name + '.atto');
    assert.equal((f.match(/\bif /g) ?? []).length, count);
    assert.equal(model(source + '\n' + f).main(), 42n);
  }
});
test('HTTP classifications and body policy agree with independent status-range oracle', () => {
  const c = model(source);
  for (let code = -1n; code <= 601n; code++) {
    assert.equal(c.http_status_valid(code), code >= 100n && code <= 599n);
    assert.equal(c.http_status_success(code), code >= 200n && code <= 299n);
    assert.equal(c.http_status_redirect(code), code >= 300n && code <= 399n);
    assert.equal(c.http_status_client_error(code), code >= 400n && code <= 499n);
    assert.equal(c.http_status_server_error(code), code >= 500n && code <= 599n);
    assert.equal(c.http_get_status_may_have_body(code), code >= 200n && code <= 599n && ![204n,205n,304n].includes(code));
  }
});
test('FS size, range, replacement and append arithmetic across signed boundaries', () => {
  const c = model(source);
  const values = [-1n,0n,1n,2n,7n,MAX-1n,MAX];
  for (const a of values) for (const b of values) for (const d of values) {
    const nonnegative = a >= 0n && b >= 0n && d >= 0n;
    assert.equal(c.fs_text_size_admissible(a,b,d), nonnegative && a <= b && a <= d);
    assert.equal(c.fs_text_range_admissible(a,b,d), nonnegative && a+b <= d);
    for (const limit of values) {
      assert.equal(c.fs_text_replacement_admissible(a,b,d,limit),
        nonnegative && limit >= 0n && a <= d && d <= limit && b+d-a <= limit);
      assert.equal(c.fs_io_append_fits_budget(a,d,b,limit,limit),
        nonnegative && limit >= 0n && a <= d && a+b <= limit && d+b <= limit);
    }
  }
});
test('mutating the copy budget predicate is detected', () => {
  const original = 'return source_bytes <= tenant_limit - tenant_used;';
  assert.ok(source.includes(original));
  assert.equal(model(source).fs_copy_fits_budget(4n,0n,3n,5n,6n), false);
  assert.equal(model(source.replace(original,'return true;')).fs_copy_fits_budget(4n,0n,3n,5n,6n), true);
});
