// Tests for the language table (LANGUAGES / MODE_SCRIPTS) in logic.js and its
// consistency with the generated index.html. Run with: node --test
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const DIST = path.join(__dirname, '..', '..', 'src-tauri', 'dist');
const L = require(path.join(DIST, 'logic.js'));
const indexHtml = fs.readFileSync(path.join(DIST, 'index.html'), 'utf8');

test('LANGUAGES rows are well-formed', () => {
    for (const lang of L.LANGUAGES) {
        assert.ok(lang.value, `value missing: ${JSON.stringify(lang)}`);
        assert.ok(lang.label, `label missing for ${lang.value}`);
        assert.ok(lang.group, `group missing for ${lang.value}`);
        assert.ok(Array.isArray(lang.exts), `exts must be an array for ${lang.value}`);
        for (const ext of lang.exts) {
            assert.equal(ext, ext.toLowerCase(), `extension must be lower-case: ${ext}`);
            assert.equal(ext.indexOf('.'), -1, `extension must not include a dot: ${ext}`);
        }
    }
});

test('language values are unique', () => {
    const seen = new Set();
    for (const lang of L.LANGUAGES) {
        assert.ok(!seen.has(lang.value), `duplicate language value: ${lang.value}`);
        seen.add(lang.value);
    }
});

test('plaintext is the first language and is present', () => {
    assert.equal(L.LANGUAGES[0].value, 'plaintext');
    assert.equal(L.modeLabel('plaintext'), 'Plain Text');
});

test('EXT_MODE resolves representative extensions', () => {
    const cases = {
        go: 'go', rs: 'rust', ts: 'application/typescript', tsx: 'jsx',
        kt: 'text/x-kotlin', scss: 'text/x-scss', cpp: 'text/x-c++src',
        yml: 'yaml', toml: 'toml', hs: 'haskell', ml: 'text/x-ocaml',
        proto: 'protobuf', tex: 'stex', vue: 'htmlmixed',
    };
    for (const [ext, value] of Object.entries(cases)) {
        assert.equal(L.extToMode(ext), value, `extToMode(${ext})`);
    }
});

test('ambiguous extension .m resolves to the first table entry (Objective-C)', () => {
    // Objective-C is listed before MATLAB/Octave, so first-definition-wins.
    assert.equal(L.extToMode('m'), 'text/x-objectivec');
});

test('modeForFilename recognises extension-less known filenames', () => {
    assert.equal(L.modeForFilename('Dockerfile'), 'dockerfile');
    assert.equal(L.modeForFilename('/srv/app/Dockerfile'), 'dockerfile');
    assert.equal(L.modeForFilename('.zshrc'), 'shell');
    assert.equal(L.modeForFilename('README'), 'plaintext'); // unknown, no extension
});

test('every EXT_MODE target and every language value has a mode label', () => {
    for (const lang of L.LANGUAGES) {
        assert.equal(L.modeLabel(lang.value), lang.label);
    }
});

test('MODE_SCRIPTS entries are unique and all files exist on disk', () => {
    const seen = new Set();
    for (const rel of L.MODE_SCRIPTS) {
        assert.ok(!seen.has(rel), `duplicate script: ${rel}`);
        seen.add(rel);
        assert.ok(fs.existsSync(path.join(DIST, rel)), `vendored script missing: ${rel}`);
    }
});

test('index.html was regenerated: one <option> per language', () => {
    const optionValues = [...indexHtml.matchAll(/<option value="([^"]*)"/g)].map(m => m[1]);
    const langValues = L.LANGUAGES.map(l => l.value);
    assert.deepEqual(
        optionValues.sort(),
        langValues.slice().sort(),
        'index.html <option> values must match LANGUAGES (run scripts/gen-index.js)',
    );
});

test('index.html loads every mode script from MODE_SCRIPTS', () => {
    for (const rel of L.MODE_SCRIPTS) {
        assert.ok(
            indexHtml.includes(`src="${rel}"`),
            `index.html is missing <script src="${rel}"> (run scripts/gen-index.js)`,
        );
    }
});

test('no label or group contains GTK-markup-breaking characters', () => {
    // On Linux the native GTK <select> popup parses option/group text as Pango
    // markup, so a bare '&', '<' or '>' produces "Failed to set text from
    // markup" warnings. Keep all rendered strings free of them.
    const bad = /[&<>]/;
    const groups = new Set();
    for (const lang of L.LANGUAGES) {
        assert.ok(!bad.test(lang.label), `label has &<> : "${lang.label}"`);
        groups.add(lang.group);
    }
    for (const group of groups) {
        assert.ok(!bad.test(group), `group has &<> : "${group}"`);
    }
});

test('languages are alphabetical (case-insensitive) within each group', () => {
    const seenGroups = [];
    const byGroup = new Map();
    for (const lang of L.LANGUAGES) {
        if (!byGroup.has(lang.group)) { byGroup.set(lang.group, []); seenGroups.push(lang.group); }
        byGroup.get(lang.group).push(lang.label);
    }
    for (const group of seenGroups) {
        const labels = byGroup.get(group);
        for (let i = 1; i < labels.length; i++) {
            const prev = labels[i - 1].toLowerCase();
            const cur = labels[i].toLowerCase();
            assert.ok(
                prev.localeCompare(cur) <= 0,
                `group "${group}" out of order: "${labels[i - 1]}" before "${labels[i]}"`,
            );
        }
    }
});

test('no file extension maps to more than one language', () => {
    // Guarantees EXT_MODE is independent of row order, so alphabetising the
    // table can never silently change which language an extension resolves to.
    const owner = {};
    for (const lang of L.LANGUAGES) {
        for (const ext of lang.exts) {
            assert.ok(!(ext in owner), `extension "${ext}" claimed by both ${owner[ext]} and ${lang.value}`);
            owner[ext] = lang.value;
        }
    }
});
