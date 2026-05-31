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
