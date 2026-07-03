// src-tauri/dist/main.js

const { invoke } = window.__TAURI__.core;
const L = window.NotepadLogic; // pure helpers from logic.js

// --- DOM ---
const openBtn = document.getElementById('openBtn');
const saveBtn = document.getElementById('saveBtn');
const saveAsBtn = document.getElementById('saveAsBtn');
const newBtn = document.getElementById('newBtn');
const findBtn = document.getElementById('findBtn');
const replaceBtn = document.getElementById('replaceBtn');
const gotoBtn = document.getElementById('gotoBtn');
const wrapBtn = document.getElementById('wrapBtn');
const themeBtn = document.getElementById('themeBtn');
const zoomInBtn = document.getElementById('zoomInBtn');
const zoomOutBtn = document.getElementById('zoomOutBtn');
const zoomResetBtn = document.getElementById('zoomResetBtn');
const languageSelect = document.getElementById('languageSelect');
const tabsContainer = document.getElementById('tabsContainer');

const statusPos = document.getElementById('statusPos');
const statusSel = document.getElementById('statusSel');
const statusLen = document.getElementById('statusLen');
const statusMode = document.getElementById('statusMode');
const statusEol = document.getElementById('statusEol');

// Find/Replace popup
const findPanel = document.getElementById('findPanel');
const findInput = document.getElementById('findInput');
const replaceRow = document.getElementById('replaceRow');
const replaceInput = document.getElementById('replaceInput');
const findPrevBtn = document.getElementById('findPrevBtn');
const findNextBtn = document.getElementById('findNextBtn');
const findCloseBtn = document.getElementById('findCloseBtn');
const replaceOneBtn = document.getElementById('replaceOneBtn');
const replaceAllBtn = document.getElementById('replaceAllBtn');
const optCase = document.getElementById('optCase');
const optRegex = document.getElementById('optRegex');
const findStatus = document.getElementById('findStatus');

// Go-to-line popup
const gotoOverlay = document.getElementById('gotoOverlay');
const gotoInput = document.getElementById('gotoInput');
const gotoGoBtn = document.getElementById('gotoGoBtn');
const gotoCancelBtn = document.getElementById('gotoCancelBtn');

// About dialog
const aboutBtn = document.getElementById('aboutBtn');
const aboutOverlay = document.getElementById('aboutOverlay');
const aboutCloseBtn = document.getElementById('aboutCloseBtn');
const aboutOkBtn = document.getElementById('aboutOkBtn');
const aboutVersion = document.getElementById('aboutVersion');

// --- Persisted settings ---
let theme = localStorage.getItem('ne.theme') || 'default'; // 'default' (light) | 'monokai' (dark)
let wrap = localStorage.getItem('ne.wrap') === '1';
let fontSize = parseInt(localStorage.getItem('ne.fontSize') || '14', 10);
if (!Number.isFinite(fontSize)) fontSize = 14;

// --- Tab state ---
let tabs = [];
let activeTabId = null;
let tabCounter = 0;

// --- Editor ---
const editor = CodeMirror(document.getElementById('editor-container'), {
    lineNumbers: true,
    theme: theme,
    lineWrapping: wrap,
    matchBrackets: true,
    styleActiveLine: true,
});

// --- Settings appliers ---
function applyTheme() {
    editor.setOption('theme', theme);
    const dark = theme === 'monokai';
    document.body.classList.toggle('dark', dark);
    // Label shows the theme you'll switch TO, so the toggle is self-explanatory.
    themeBtn.textContent = dark ? '☀ Light' : '🌙 Dark';
    themeBtn.title = dark ? 'Switch to light theme' : 'Switch to dark theme';
    localStorage.setItem('ne.theme', theme);
}
function applyWrap() {
    editor.setOption('lineWrapping', wrap);
    wrapBtn.setAttribute('aria-pressed', wrap ? 'true' : 'false');
    wrapBtn.classList.toggle('active', wrap);
    localStorage.setItem('ne.wrap', wrap ? '1' : '0');
}
function applyFontSize() {
    fontSize = Math.max(8, Math.min(40, fontSize));
    editor.getWrapperElement().style.fontSize = fontSize + 'px';
    editor.refresh();
    localStorage.setItem('ne.fontSize', String(fontSize));
}

// --- Status bar ---
function updatePos() {
    const c = editor.getCursor();
    statusPos.textContent = `Ln ${c.line + 1}, Col ${c.ch + 1}`;
    statusSel.textContent = `Sel ${editor.getSelections().join('').length}`;
}
function updateDocStats() {
    statusLen.textContent = `Length ${editor.getValue().length}, Lines ${editor.lineCount()}`;
}
function updateMeta() {
    const tab = getActiveTab();
    statusMode.textContent = tab ? L.modeLabel(tab.mode) : 'Plain Text';
    statusEol.textContent = tab ? tab.eol : 'LF';
}

editor.on('cursorActivity', updatePos);
editor.on('changes', updateDocStats);

// --- Tabs ---
function createTab(name, path, content, modeValue, eol) {
    tabCounter++;
    const id = tabCounter;
    const doc = CodeMirror.Doc(content, L.resolveMode(modeValue));
    doc.markClean();
    CodeMirror.on(doc, 'change', renderTabs);
    tabs.push({ id, name, path, doc, mode: modeValue, eol: eol || 'LF' });
    switchTab(id);
}

function switchTab(id) {
    activeTabId = id;
    const tab = tabs.find(t => t.id === id);
    if (tab) {
        editor.swapDoc(tab.doc);
        editor.setOption('mode', L.resolveMode(tab.mode));
        languageSelect.value = tab.mode;
        document.title = tab.path ? `Notepad Extra - ${tab.path}` : 'Notepad Extra - Untitled';
        updatePos();
        updateDocStats();
        updateMeta();
    }
    renderTabs();
}

function closeTab(id, event) {
    event.stopPropagation();
    const tab = tabs.find(t => t.id === id);
    if (!tab) return;
    if (!tab.doc.isClean()) {
        const confirmClose = confirm(`"${tab.name}" has unsaved changes. Close without saving?`);
        if (!confirmClose) return;
    }
    tabs = tabs.filter(t => t.id !== id);
    if (tabs.length === 0) {
        createTab('Untitled', null, '', 'plaintext', 'LF');
    } else if (activeTabId === id) {
        switchTab(tabs[tabs.length - 1].id);
    } else {
        renderTabs();
    }
}

function renderTabs() {
    tabsContainer.innerHTML = '';
    tabs.forEach(tab => {
        const tabEl = document.createElement('div');
        tabEl.className = `tab ${tab.id === activeTabId ? 'active' : ''}`;
        const isDirty = !tab.doc.isClean();
        tabEl.textContent = tab.name + (isDirty ? ' •' : '');

        const closeBtn = document.createElement('span');
        closeBtn.className = 'tab-close';
        closeBtn.textContent = '×';
        closeBtn.onclick = (e) => closeTab(tab.id, e);

        tabEl.appendChild(closeBtn);
        tabEl.onclick = () => switchTab(tab.id);
        tabsContainer.appendChild(tabEl);
    });
}

function getActiveTab() { return tabs.find(t => t.id === activeTabId); }

function applySaveResult(tab, result) {
    if (result && result.path) {
        tab.path = result.path;
        tab.name = L.basename(result.path);
        document.title = `Notepad Extra - ${tab.path}`;
        tab.doc.markClean();
        renderTabs();
    }
}

// --- File commands ---
async function doOpen() {
    try {
        const result = await invoke('open_file');
        if (result) {
            createTab(
                L.basename(result.path), result.path, result.content,
                L.modeForFilename(result.path), L.detectEol(result.content),
            );
        }
    } catch (error) {
        console.error('Error opening file:', error);
    }
}

async function doSave() {
    const tab = getActiveTab();
    if (!tab) return;
    try {
        const content = L.eolJoin(editor.getValue(), tab.eol);
        const result = await invoke('save_file', { content, path: tab.path });
        applySaveResult(tab, result);
    } catch (error) {
        console.error('Error saving file:', error);
    }
}

async function doSaveAs() {
    const tab = getActiveTab();
    if (!tab) return;
    try {
        const content = L.eolJoin(editor.getValue(), tab.eol);
        const result = await invoke('save_file_as', { content });
        applySaveResult(tab, result);
    } catch (error) {
        console.error('Error saving file:', error);
    }
}

// --- Find / Replace popup ---
function findIsOpen() { return !findPanel.classList.contains('hidden'); }

function openFind(showReplace) {
    replaceRow.classList.toggle('hidden', !showReplace);
    const sel = editor.getSelection();
    if (sel && sel.indexOf('\n') === -1) findInput.value = sel;
    findPanel.classList.remove('hidden');
    findStatus.textContent = '';
    findInput.focus();
    findInput.select();
}

function closeFind() {
    findPanel.classList.add('hidden');
    editor.focus();
}

function currentQuery() {
    return L.buildSearchQuery(findInput.value, {
        regex: optRegex.checked,
        caseSensitive: optCase.checked,
    });
}

function doFind(forward) {
    const query = currentQuery();
    if (!query) { findStatus.textContent = ''; return false; }

    const start = forward ? editor.getCursor('to') : editor.getCursor('from');
    let cursor = editor.getSearchCursor(query, start);
    let found = forward ? cursor.findNext() : cursor.findPrevious();

    if (!found) {
        // Wrap around to the other end of the document.
        const last = editor.lastLine();
        const wrapStart = forward
            ? { line: 0, ch: 0 }
            : { line: last, ch: editor.getLine(last).length };
        cursor = editor.getSearchCursor(query, wrapStart);
        found = forward ? cursor.findNext() : cursor.findPrevious();
    }

    if (found) {
        editor.setSelection(cursor.from(), cursor.to());
        editor.scrollIntoView({ from: cursor.from(), to: cursor.to() }, 80);
        findStatus.textContent = '';
    } else {
        findStatus.textContent = 'No matches';
    }
    return found;
}

function doReplaceOne() {
    const query = currentQuery();
    if (!query) return;
    const cursor = editor.getSearchCursor(query, editor.getCursor('from'));
    if (cursor.findNext()) {
        editor.setSelection(cursor.from(), cursor.to());
        cursor.replace(replaceInput.value);
    }
    doFind(true);
}

function doReplaceAll() {
    const query = currentQuery();
    if (!query) return;
    let count = 0;
    editor.operation(() => {
        const cursor = editor.getSearchCursor(query, { line: 0, ch: 0 });
        while (cursor.findNext()) {
            cursor.replace(replaceInput.value);
            count++;
        }
    });
    findStatus.textContent = `Replaced ${count}`;
}

// --- Go-to-line popup ---
function openGoto() {
    gotoInput.value = String(editor.getCursor().line + 1);
    gotoOverlay.classList.remove('hidden');
    gotoInput.focus();
    gotoInput.select();
}

function closeGoto() {
    gotoOverlay.classList.add('hidden');
    editor.focus();
}

function doGoto() {
    const line = L.clampLine(gotoInput.value, editor.lineCount());
    if (line !== null) {
        const pos = { line: line - 1, ch: 0 };
        editor.setCursor(pos);
        editor.scrollIntoView(pos, 100);
    }
    closeGoto();
}

// --- About dialog ---
let aboutVersionLoaded = false;

async function openAbout() {
    aboutOverlay.classList.remove('hidden');
    if (!aboutVersionLoaded) {
        try {
            aboutVersion.textContent = await invoke('app_version');
        } catch (error) {
            aboutVersion.textContent = 'unknown';
            console.error('Could not read app version:', error);
        }
        aboutVersionLoaded = true;
    }
    aboutOkBtn.focus();
}

function closeAbout() {
    aboutOverlay.classList.add('hidden');
    editor.focus();
}

function aboutIsOpen() { return !aboutOverlay.classList.contains('hidden'); }

// Open external https links in the user's own browser (the app never fetches
// anything itself). URLs are declared via data-url on .ext-link elements.
async function openExternal(url) {
    try {
        await invoke('open_external', { url });
    } catch (error) {
        console.error('Could not open URL:', url, error);
    }
}

document.querySelectorAll('.ext-link').forEach((el) => {
    el.addEventListener('click', (e) => {
        e.preventDefault();
        const url = el.getAttribute('data-url');
        if (url) openExternal(url);
    });
});

// --- Event listeners ---
languageSelect.addEventListener('change', () => {
    const tab = getActiveTab();
    if (tab) {
        tab.mode = languageSelect.value;
        editor.setOption('mode', L.resolveMode(tab.mode));
        updateMeta();
    }
});

newBtn.addEventListener('click', () => createTab('Untitled', null, '', 'plaintext', 'LF'));
openBtn.addEventListener('click', doOpen);
saveBtn.addEventListener('click', doSave);
saveAsBtn.addEventListener('click', doSaveAs);

findBtn.addEventListener('click', () => openFind(false));
replaceBtn.addEventListener('click', () => openFind(true));
gotoBtn.addEventListener('click', openGoto);

findNextBtn.addEventListener('click', () => doFind(true));
findPrevBtn.addEventListener('click', () => doFind(false));
findCloseBtn.addEventListener('click', closeFind);
replaceOneBtn.addEventListener('click', doReplaceOne);
replaceAllBtn.addEventListener('click', doReplaceAll);

findInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); doFind(!e.shiftKey); }
    else if (e.key === 'Escape') { e.preventDefault(); closeFind(); }
});
replaceInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); doReplaceOne(); }
    else if (e.key === 'Escape') { e.preventDefault(); closeFind(); }
});

gotoGoBtn.addEventListener('click', doGoto);
gotoCancelBtn.addEventListener('click', closeGoto);
gotoInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); doGoto(); }
    else if (e.key === 'Escape') { e.preventDefault(); closeGoto(); }
});
gotoOverlay.addEventListener('mousedown', (e) => { if (e.target === gotoOverlay) closeGoto(); });

aboutBtn.addEventListener('click', openAbout);
aboutCloseBtn.addEventListener('click', closeAbout);
aboutOkBtn.addEventListener('click', closeAbout);
aboutOverlay.addEventListener('mousedown', (e) => { if (e.target === aboutOverlay) closeAbout(); });

wrapBtn.addEventListener('click', () => { wrap = !wrap; applyWrap(); });
themeBtn.addEventListener('click', () => { theme = theme === 'monokai' ? 'default' : 'monokai'; applyTheme(); });
zoomInBtn.addEventListener('click', () => { fontSize += 1; applyFontSize(); });
zoomOutBtn.addEventListener('click', () => { fontSize -= 1; applyFontSize(); });
zoomResetBtn.addEventListener('click', () => { fontSize = 14; applyFontSize(); });

// Global shortcuts.
document.addEventListener('keydown', (e) => {
    if (e.key === 'F3') { e.preventDefault(); doFind(!e.shiftKey); return; }
    if (e.key === 'Escape') {
        if (aboutIsOpen()) { closeAbout(); return; }
        if (findIsOpen()) { closeFind(); return; }
        if (!gotoOverlay.classList.contains('hidden')) { closeGoto(); return; }
    }
    if (!(e.ctrlKey || e.metaKey)) return;
    switch (e.key.toLowerCase()) {
        case 's': e.preventDefault(); e.shiftKey ? doSaveAs() : doSave(); break;
        case 'o': e.preventDefault(); doOpen(); break;
        case 'n': e.preventDefault(); newBtn.click(); break;
        case 'f': e.preventDefault(); openFind(false); break;
        case 'h': e.preventDefault(); openFind(true); break;
        case 'g': e.preventDefault(); openGoto(); break;
        case '=': case '+': e.preventDefault(); fontSize += 1; applyFontSize(); break;
        case '-': case '_': e.preventDefault(); fontSize -= 1; applyFontSize(); break;
        case '0': e.preventDefault(); fontSize = 14; applyFontSize(); break;
    }
});

// --- Init ---
applyTheme();
applyWrap();
applyFontSize();
createTab('Untitled', null, '', 'plaintext', 'LF');
