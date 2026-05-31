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
    extraKeys: {
        'Ctrl-F': 'find', 'Cmd-F': 'find',
        'Ctrl-H': 'replace', 'Cmd-H': 'replace',
        'Ctrl-G': 'jumpToLine', 'Cmd-G': 'jumpToLine',
        'F3': 'findNext', 'Shift-F3': 'findPrev',
    },
});

// --- Settings appliers ---
function applyTheme() {
    editor.setOption('theme', theme);
    document.body.classList.toggle('dark', theme === 'monokai');
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

findBtn.addEventListener('click', () => { editor.focus(); editor.execCommand('find'); });
replaceBtn.addEventListener('click', () => { editor.focus(); editor.execCommand('replace'); });
gotoBtn.addEventListener('click', () => { editor.focus(); editor.execCommand('jumpToLine'); });

wrapBtn.addEventListener('click', () => { wrap = !wrap; applyWrap(); });
themeBtn.addEventListener('click', () => { theme = theme === 'monokai' ? 'default' : 'monokai'; applyTheme(); });
zoomInBtn.addEventListener('click', () => { fontSize += 1; applyFontSize(); });
zoomOutBtn.addEventListener('click', () => { fontSize -= 1; applyFontSize(); });
zoomResetBtn.addEventListener('click', () => { fontSize = 14; applyFontSize(); });

// Global shortcuts for file ops and zoom (find/replace/goto live in editor extraKeys).
document.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const key = e.key.toLowerCase();
    if (key === 's') { e.preventDefault(); e.shiftKey ? doSaveAs() : doSave(); }
    else if (key === 'o') { e.preventDefault(); doOpen(); }
    else if (key === 'n') { e.preventDefault(); newBtn.click(); }
    else if (key === '=' || key === '+') { e.preventDefault(); fontSize += 1; applyFontSize(); }
    else if (key === '-' || key === '_') { e.preventDefault(); fontSize -= 1; applyFontSize(); }
    else if (key === '0') { e.preventDefault(); fontSize = 14; applyFontSize(); }
});

// --- Init ---
applyTheme();
applyWrap();
applyFontSize();
createTab('Untitled', null, '', 'plaintext', 'LF');
