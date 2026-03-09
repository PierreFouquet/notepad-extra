// src-tauri/dist/main.js

const { invoke } = window.__TAURI__.core;

const openBtn = document.getElementById('openBtn');
const saveBtn = document.getElementById('saveBtn');
const newBtn = document.getElementById('newBtn');
const languageSelect = document.getElementById('languageSelect');
const tabsContainer = document.getElementById('tabsContainer');

// Tab State Management
let tabs = [];
let activeTabId = null;
let tabCounter = 0;

// Initialize CodeMirror 
const editor = CodeMirror(document.getElementById('editor-container'), {
    lineNumbers: true,
    theme: 'monokai'
});

// Tab Functions
function createTab(name, path, content, mode) {
    tabCounter++;
    const id = tabCounter;
    const doc = CodeMirror.Doc(content, mode);
    
    // Mark the document as unmodified from the start
    doc.markClean();
    
    // Listen for typing so we can update the tab to show the "Unsaved" indicator
    CodeMirror.on(doc, 'change', () => {
        renderTabs();
    });

    tabs.push({ id, name, path, doc, mode });
    switchTab(id);
}

function switchTab(id) {
    activeTabId = id;
    const tab = tabs.find(t => t.id === id);
    if (tab) {
        editor.swapDoc(tab.doc);
        languageSelect.value = tab.mode;
        document.title = tab.path ? `Notepad Extra - ${tab.path}` : 'Notepad Extra - Untitled';
    }
    renderTabs();
}

function closeTab(id, event) {
    event.stopPropagation(); // Prevent switching to the tab while closing it
    
    const tab = tabs.find(t => t.id === id);
    if (!tab) return;

    // THE FIX: Unsaved Changes Warning
    if (!tab.doc.isClean()) {
        const confirmClose = confirm(`"${tab.name}" has unsaved changes. Are you sure you want to close it without saving?`);
        if (!confirmClose) return; // Abort the close if they click Cancel
    }

    tabs = tabs.filter(t => t.id !== id);
    if (tabs.length === 0) {
        createTab('Untitled', null, '', 'plaintext');
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
        
        // Add a visual indicator (•) if the document has unsaved changes
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

function getActiveTab() {
    return tabs.find(t => t.id === activeTabId);
}

// Event Listeners
languageSelect.addEventListener('change', () => {
    const tab = getActiveTab();
    if (tab) {
        tab.mode = languageSelect.value;
        editor.setOption('mode', tab.mode);
    }
});

newBtn.addEventListener('click', () => {
    createTab('Untitled', null, '', 'plaintext');
});

openBtn.addEventListener('click', async () => {
    try {
        const result = await invoke('open_file');
        if (result) {
            const ext = result.path.split('.').pop().toLowerCase();
            const modeMap = {
                'js': 'javascript', 'rs': 'rust', 'md': 'markdown',
                'html': 'htmlmixed', 'css': 'css', 'txt': 'plaintext'
            };
            const mode = modeMap[ext] || 'plaintext';
            const filename = result.path.split(/[/\\]/).pop();
            
            createTab(filename, result.path, result.content, mode);
        }
    } catch (error) {
        console.error('Error opening file:', error);
    }
});

saveBtn.addEventListener('click', async () => {
    const tab = getActiveTab();
    if (!tab) return;
    
    try {
        const content = editor.getValue();
        const result = await invoke('save_file', { content, path: tab.path });
        if (result && result.path) {
            tab.path = result.path;
            tab.name = result.path.split(/[/\\]/).pop();
            document.title = `Notepad Extra - ${tab.path}`;
            
            // Mark the document as clean so the warning disappears
            tab.doc.markClean(); 
            renderTabs();
        }
    } catch (error) {
        console.error('Error saving file:', error);
    }
});

// IMPROVEMENT: Global Keyboard Shortcuts
document.addEventListener('keydown', (e) => {
    // Check for Ctrl (Windows/Linux) or Cmd (Mac)
    if (e.ctrlKey || e.metaKey) {
        if (e.key === 's' || e.key === 'S') {
            e.preventDefault(); // Stop the browser's default "Save Webpage" dialog
            saveBtn.click();
        }
        if (e.key === 'o' || e.key === 'O') {
            e.preventDefault();
            openBtn.click();
        }
        if (e.key === 'n' || e.key === 'N') {
            e.preventDefault();
            newBtn.click();
        }
    }
});

// Start with one empty tab
createTab('Untitled', null, '', 'plaintext');