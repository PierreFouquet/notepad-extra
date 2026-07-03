// Tests that the Linux packaging metadata (issue #15) stays internally
// consistent: one reverse-DNS app-id shared across tauri.conf.json, the
// AppStream metainfo, the .desktop file and the bundle file maps, and that every
// file the Tauri bundle installs actually exists in the tree. Run with: node --test
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.join(__dirname, '..', '..');
const PKG = path.join(ROOT, 'packaging', 'linux');
const APP_ID = 'io.github.PierreFouquet.NotepadExtra';

const read = (p) => fs.readFileSync(p, 'utf8');
const tauriConf = JSON.parse(read(path.join(ROOT, 'tauri.conf.json')));
const desktopPath = path.join(PKG, `${APP_ID}.desktop`);
const metainfoPath = path.join(PKG, `${APP_ID}.metainfo.xml`);
const desktop = read(desktopPath);
const metainfo = read(metainfoPath);
const cargoToml = read(path.join(ROOT, 'Cargo.toml'));

// Parse the [Desktop Entry] group of a .desktop file into a key→value object.
function desktopEntries(text) {
    const out = {};
    let inEntry = false;
    for (const line of text.split(/\r?\n/)) {
        const t = line.trim();
        if (t.startsWith('[')) { inEntry = t === '[Desktop Entry]'; continue; }
        if (!inEntry || !t || t.startsWith('#')) continue;
        const i = t.indexOf('=');
        if (i > 0) out[t.slice(0, i)] = t.slice(i + 1);
    }
    return out;
}
const entry = desktopEntries(desktop);

test('tauri.conf.json identifier is the reverse-DNS app-id', () => {
    assert.equal(tauriConf.identifier, APP_ID);
    assert.ok(!APP_ID.includes('-'), 'app-id must not contain a hyphen (invalid D-Bus/AppStream id)');
});

test('.desktop is well-formed and keyed to the app-id', () => {
    assert.equal(entry.Type, 'Application');
    assert.equal(entry.Icon, APP_ID, 'Icon must match the app-id so hicolor icons resolve');
    assert.ok(entry.Name, 'Name is required');
    assert.ok(/^notepad-extra(\s|$)/.test(entry.Exec), `Exec should launch the notepad-extra binary, got: ${entry.Exec}`);
    for (const key of ['Categories', 'Keywords', 'MimeType']) {
        assert.ok(entry[key], `${key} is required`);
        assert.ok(entry[key].endsWith(';'), `${key} list must be semicolon-terminated`);
    }
    assert.ok(entry.Categories.split(';').includes('TextEditor'), 'should register as a TextEditor');
    assert.ok(entry.MimeType.split(';').includes('text/plain'), 'should handle text/plain');
});

test('AppStream metainfo matches the app-id, .desktop and license', () => {
    assert.match(metainfo, new RegExp(`<id>${APP_ID.replace(/\./g, '\\.')}</id>`));
    assert.match(metainfo, new RegExp(`<launchable type="desktop-id">${APP_ID.replace(/\./g, '\\.')}\\.desktop</launchable>`));
    assert.match(metainfo, /<project_license>GPL-3\.0-or-later<\/project_license>/);
    assert.match(metainfo, /<metadata_license>/);
    assert.match(metainfo, /<developer id="[a-z0-9.]+">/, 'developer id must be lowercase (else appstreamcli warns)');
});

test('metainfo release version tracks the app version', () => {
    const m = metainfo.match(/<release version="([^"]+)"/);
    assert.ok(m, 'metainfo must declare a release');
    assert.equal(m[1], tauriConf.version, 'metainfo release must match tauri.conf.json version');
});

test('man page exists and is a section-1 page', () => {
    const man = read(path.join(PKG, 'notepad-extra.1'));
    assert.match(man, /^\.TH NOTEPAD-EXTRA 1/m);
});

test('bundle-level license and category are declared', () => {
    assert.equal(tauriConf.bundle.license, 'GPL-3.0-or-later');
    assert.ok(tauriConf.bundle.category, 'bundle.category should be set');
    assert.ok(tauriConf.bundle.longDescription, 'bundle.longDescription should be set');
});

test('every deb/rpm bundle file source exists and desktopTemplate resolves', () => {
    for (const target of ['deb', 'rpm']) {
        const cfg = tauriConf.bundle.linux[target];
        assert.ok(cfg, `bundle.linux.${target} must exist`);
        assert.ok(Array.isArray(cfg.depends) && cfg.depends.length > 0, `${target} depends should be listed`);

        assert.equal(cfg.desktopTemplate, `packaging/linux/${APP_ID}.desktop`);
        assert.ok(fs.existsSync(path.join(ROOT, cfg.desktopTemplate)), `${target} desktopTemplate source is missing`);

        const files = cfg.files || {};
        assert.ok(Object.keys(files).length > 0, `${target} should install packaging files`);
        for (const [dest, src] of Object.entries(files)) {
            assert.ok(fs.existsSync(path.join(ROOT, src)), `${target} file source missing: ${src}`);
            if (dest.includes('/icons/hicolor/')) {
                assert.ok(dest.endsWith(`/${APP_ID}.png`), `hicolor icon must be named by app-id: ${dest}`);
            }
        }
        // The metainfo must be installed under /usr/share/metainfo with the app-id name.
        assert.ok(
            Object.keys(files).some((d) => d === `/usr/share/metainfo/${APP_ID}.metainfo.xml`),
            `${target} must install the AppStream metainfo`,
        );
    }
});

test('Cargo.toml declares the GPL-3.0-or-later license', () => {
    assert.match(cargoToml, /^license\s*=\s*"GPL-3\.0-or-later"/m);
});
