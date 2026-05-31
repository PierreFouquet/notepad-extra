// src-tauri/dist/logic.js
//
// Pure, dependency-free helpers shared by the UI (main.js) and the test suite.
// Loaded as a plain browser script (exposes `window.NotepadLogic`) and is also
// require()-able from Node for unit testing (`module.exports`).

(function (factory) {
    const api = factory();
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
    if (typeof window !== 'undefined') window.NotepadLogic = api;
})(function () {
    // Internal value -> human label. Values are CodeMirror mode names or MIME types.
    const LANG_LABELS = {
        'plaintext': 'Plain Text',
        'javascript': 'JavaScript',
        'application/json': 'JSON',
        'rust': 'Rust',
        'python': 'Python',
        'text/x-csrc': 'C',
        'text/x-c++src': 'C++',
        'text/x-java': 'Java',
        'markdown': 'Markdown',
        'htmlmixed': 'HTML',
        'xml': 'XML',
        'css': 'CSS',
        'shell': 'Shell',
        'yaml': 'YAML',
    };

    // File extension (lower-case, no dot) -> language value.
    const EXT_MODE = {
        js: 'javascript', mjs: 'javascript', cjs: 'javascript', ts: 'javascript', jsx: 'javascript',
        json: 'application/json',
        rs: 'rust',
        py: 'python', pyw: 'python',
        c: 'text/x-csrc', h: 'text/x-csrc',
        cpp: 'text/x-c++src', cc: 'text/x-c++src', cxx: 'text/x-c++src', hpp: 'text/x-c++src', hxx: 'text/x-c++src',
        java: 'text/x-java',
        md: 'markdown', markdown: 'markdown',
        html: 'htmlmixed', htm: 'htmlmixed',
        xml: 'xml', svg: 'xml', xaml: 'xml',
        css: 'css',
        sh: 'shell', bash: 'shell', zsh: 'shell',
        yml: 'yaml', yaml: 'yaml',
    };

    function modeLabel(value) {
        return LANG_LABELS[value] || 'Plain Text';
    }

    // CodeMirror uses `null` for "no highlighting" (plain text).
    function resolveMode(value) {
        return value === 'plaintext' ? null : value;
    }

    // Map a file extension to a language value (defaults to plain text).
    function extToMode(ext) {
        if (!ext) return 'plaintext';
        return EXT_MODE[String(ext).toLowerCase()] || 'plaintext';
    }

    // Map a full path/filename to a language value via its extension.
    function modeForFilename(path) {
        if (!path) return 'plaintext';
        const name = String(path).split(/[/\\]/).pop();
        if (name.indexOf('.') === -1) return 'plaintext';
        return extToMode(name.split('.').pop());
    }

    // Extract just the file name from a path.
    function basename(path) {
        return String(path).split(/[/\\]/).pop();
    }

    // Detect a file's end-of-line style from its content.
    function detectEol(content) {
        return String(content).indexOf('\r\n') !== -1 ? 'CRLF' : 'LF';
    }

    // CodeMirror stores text with '\n'; re-join with the file's EOL on save.
    function eolJoin(text, eol) {
        return eol === 'CRLF' ? String(text).replace(/\n/g, '\r\n') : String(text);
    }

    return {
        LANG_LABELS, EXT_MODE,
        modeLabel, resolveMode, extToMode, modeForFilename, basename, detectEol, eolJoin,
    };
});
