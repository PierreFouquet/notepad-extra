// Unit tests for the pure frontend logic (src-tauri/dist/logic.js).
// Run with: node --test
const test = require('node:test');
const assert = require('node:assert/strict');
const L = require('../../src-tauri/dist/logic.js');

test('resolveMode maps plaintext to null, passes others through', () => {
    assert.equal(L.resolveMode('plaintext'), null);
    assert.equal(L.resolveMode('rust'), 'rust');
    assert.equal(L.resolveMode('application/json'), 'application/json');
});

test('modeLabel returns friendly names and defaults to Plain Text', () => {
    assert.equal(L.modeLabel('rust'), 'Rust');
    assert.equal(L.modeLabel('application/json'), 'JSON');
    assert.equal(L.modeLabel('text/x-c++src'), 'C++');
    assert.equal(L.modeLabel('plaintext'), 'Plain Text');
    assert.equal(L.modeLabel('totally-unknown'), 'Plain Text');
});

test('extToMode is case-insensitive and falls back to plaintext', () => {
    assert.equal(L.extToMode('rs'), 'rust');
    assert.equal(L.extToMode('RS'), 'rust');
    assert.equal(L.extToMode('py'), 'python');
    assert.equal(L.extToMode('cpp'), 'text/x-c++src');
    assert.equal(L.extToMode('json'), 'application/json');
    assert.equal(L.extToMode('weird'), 'plaintext');
    assert.equal(L.extToMode(''), 'plaintext');
    assert.equal(L.extToMode(undefined), 'plaintext');
});

test('modeForFilename handles unix/windows paths, case and missing extensions', () => {
    assert.equal(L.modeForFilename('/home/u/main.rs'), 'rust');
    assert.equal(L.modeForFilename('C:\\proj\\app.PY'), 'python');
    assert.equal(L.modeForFilename('index.html'), 'htmlmixed');
    assert.equal(L.modeForFilename('data.JSON'), 'application/json');
    assert.equal(L.modeForFilename('/etc/hostname'), 'plaintext'); // no extension
    assert.equal(L.modeForFilename('archive.tar.gz'), 'plaintext'); // unknown ext
    assert.equal(L.modeForFilename(''), 'plaintext');
    assert.equal(L.modeForFilename(null), 'plaintext');
});

test('basename extracts the file name from unix and windows paths', () => {
    assert.equal(L.basename('/a/b/c.txt'), 'c.txt');
    assert.equal(L.basename('C:\\a\\b.txt'), 'b.txt');
    assert.equal(L.basename('file.txt'), 'file.txt');
});

test('detectEol recognises CRLF, LF and empty content', () => {
    assert.equal(L.detectEol('a\r\nb'), 'CRLF');
    assert.equal(L.detectEol('a\nb'), 'LF');
    assert.equal(L.detectEol('no newline'), 'LF');
    assert.equal(L.detectEol(''), 'LF');
    assert.equal(L.detectEol('lone\rcr'), 'LF'); // bare CR is not treated as CRLF
});

test('eolJoin converts to CRLF only when requested and is otherwise a no-op', () => {
    assert.equal(L.eolJoin('a\nb\nc', 'CRLF'), 'a\r\nb\r\nc');
    assert.equal(L.eolJoin('a\nb\nc', 'LF'), 'a\nb\nc');
    assert.equal(L.eolJoin('a\nb', undefined), 'a\nb'); // default is LF (no conversion)
    assert.equal(L.eolJoin('', 'CRLF'), '');
});

test('round-trip: detected EOL re-applied by eolJoin reproduces CRLF files', () => {
    // CodeMirror normalises input to \n; eolJoin must restore the original EOL.
    const original = 'one\r\ntwo\r\nthree';
    const eol = L.detectEol(original);
    const normalised = original.replace(/\r\n/g, '\n'); // what the editor would hold
    assert.equal(L.eolJoin(normalised, eol), original);
});

test('every EXT_MODE target has a human label', () => {
    for (const value of new Set(Object.values(L.EXT_MODE))) {
        assert.ok(L.LANG_LABELS[value], `missing label for language value "${value}"`);
    }
});

test('escapeRegExp escapes regex metacharacters', () => {
    assert.equal(L.escapeRegExp('a.b*c'), 'a\\.b\\*c');
    assert.equal(L.escapeRegExp('(x)[y]{z}'), '\\(x\\)\\[y\\]\\{z\\}');
    assert.equal(L.escapeRegExp('plain'), 'plain');
});

test('buildSearchQuery treats plain text literally', () => {
    const q = L.buildSearchQuery('a.b', { regex: false });
    assert.ok(q instanceof RegExp);
    assert.ok(q.test('a.b'));
    assert.ok(!q.test('axb')); // the dot is literal, not "any char"
});

test('buildSearchQuery honours case sensitivity', () => {
    assert.ok(L.buildSearchQuery('abc', { caseSensitive: false }).test('ABC'));
    assert.ok(!L.buildSearchQuery('abc', { caseSensitive: true }).test('ABC'));
});

test('buildSearchQuery compiles regex mode and rejects empties/invalid patterns', () => {
    assert.ok(L.buildSearchQuery('a.c', { regex: true }).test('abc'));
    assert.equal(L.buildSearchQuery('', {}), null);
    assert.equal(L.buildSearchQuery('(unclosed', { regex: true }), null);
});

test('clampLine bounds the target line and rejects non-numbers', () => {
    assert.equal(L.clampLine(5, 10), 5);
    assert.equal(L.clampLine(0, 10), 1);     // floor at 1
    assert.equal(L.clampLine(999, 10), 10);  // ceil at lineCount
    assert.equal(L.clampLine('3', 10), 3);   // numeric strings ok
    assert.equal(L.clampLine('abc', 10), null);
});

test('tabDescriptor maps a read result to createTab fields', () => {
    const d = L.tabDescriptor({ path: '/home/u/main.rs', content: 'fn main() {}\n' });
    assert.equal(d.name, 'main.rs');
    assert.equal(d.path, '/home/u/main.rs');
    assert.equal(d.content, 'fn main() {}\n');
    assert.equal(d.mode, 'rust');
    assert.equal(d.eol, 'LF');
});

test('tabDescriptor detects language and CRLF, and handles Windows paths', () => {
    const d = L.tabDescriptor({ path: 'C:\\Users\\u\\notes.md', content: 'a\r\nb\r\n' });
    assert.equal(d.name, 'notes.md');
    assert.equal(d.mode, 'markdown');
    assert.equal(d.eol, 'CRLF');
});

test('tabDescriptor falls back to plaintext for unknown extensions', () => {
    const d = L.tabDescriptor({ path: '/tmp/file.unknownext', content: '' });
    assert.equal(d.mode, 'plaintext');
    assert.equal(d.eol, 'LF');
});

test('shouldReuseBlankTab: reuse a lone pristine Untitled tab', () => {
    assert.equal(
        L.shouldReuseBlankTab(1, { hasPath: false, isClean: true, isEmpty: true }),
        true,
    );
});

test('shouldReuseBlankTab: never reuse when other tabs are open', () => {
    assert.equal(
        L.shouldReuseBlankTab(2, { hasPath: false, isClean: true, isEmpty: true }),
        false,
    );
});

test('shouldReuseBlankTab: never reuse a saved, dirty, or non-empty tab', () => {
    assert.equal(L.shouldReuseBlankTab(1, { hasPath: true, isClean: true, isEmpty: true }), false);
    assert.equal(L.shouldReuseBlankTab(1, { hasPath: false, isClean: false, isEmpty: true }), false);
    assert.equal(L.shouldReuseBlankTab(1, { hasPath: false, isClean: true, isEmpty: false }), false);
});

test('shouldReuseBlankTab: safe with a missing tab snapshot', () => {
    assert.equal(L.shouldReuseBlankTab(1, null), false);
});
