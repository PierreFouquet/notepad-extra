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
    // ------------------------------------------------------------------
    // Single source of truth for language support.
    //
    // Every language the app knows about is one row here. From this table we
    // derive the human labels, the file-extension -> language map, and (via
    // scripts/gen-index.js) both the toolbar dropdown and the <script> tags
    // that load the matching CodeMirror mode. Add a language in ONE place.
    //
    //   value : the string passed to CodeMirror (a mode name or a MIME type).
    //           'plaintext' is special-cased to "no highlighting".
    //   label : what the user sees in the dropdown / status bar.
    //   group : dropdown optgroup heading.
    //   exts  : file extensions (lower-case, no dot) that auto-select it.
    // ------------------------------------------------------------------
    //
    // Rows are kept alphabetical by label within each group (the tests enforce
    // this). Group headings avoid '&', '<' and '>' because on Linux the native
    // GTK <select> popup parses option text as Pango markup and a bare '&'
    // triggers "Failed to set text from markup" warnings.
    const LANGUAGES = [
        { value: 'plaintext', label: 'Plain Text', group: 'General', exts: ['txt', 'text', 'log'] },

        // --- Popular ---
        { value: 'text/x-csrc', label: 'C', group: 'Popular', exts: ['c', 'h'] },
        { value: 'text/x-csharp', label: 'C#', group: 'Popular', exts: ['cs'] },
        { value: 'text/x-c++src', label: 'C++', group: 'Popular', exts: ['cpp', 'cc', 'cxx', 'c++', 'hpp', 'hxx', 'hh'] },
        { value: 'go', label: 'Go', group: 'Popular', exts: ['go'] },
        { value: 'text/x-java', label: 'Java', group: 'Popular', exts: ['java'] },
        { value: 'javascript', label: 'JavaScript', group: 'Popular', exts: ['js', 'mjs', 'cjs'] },
        { value: 'jsx', label: 'JSX / TSX (React)', group: 'Popular', exts: ['jsx', 'tsx'] },
        { value: 'text/x-kotlin', label: 'Kotlin', group: 'Popular', exts: ['kt', 'kts'] },
        { value: 'text/x-objectivec', label: 'Objective-C', group: 'Popular', exts: ['m', 'mm'] },
        { value: 'php', label: 'PHP', group: 'Popular', exts: ['php', 'phtml', 'php3', 'php4', 'php5'] },
        { value: 'python', label: 'Python', group: 'Popular', exts: ['py', 'pyw', 'pyi'] },
        { value: 'ruby', label: 'Ruby', group: 'Popular', exts: ['rb', 'gemspec'] },
        { value: 'rust', label: 'Rust', group: 'Popular', exts: ['rs'] },
        { value: 'text/x-scala', label: 'Scala', group: 'Popular', exts: ['scala', 'sc'] },
        { value: 'swift', label: 'Swift', group: 'Popular', exts: ['swift'] },
        { value: 'application/typescript', label: 'TypeScript', group: 'Popular', exts: ['ts', 'mts', 'cts'] },

        // --- Web / Markup ---
        { value: 'css', label: 'CSS', group: 'Web / Markup', exts: ['css'] },
        { value: 'django', label: 'Django Template', group: 'Web / Markup', exts: [] },
        { value: 'handlebars', label: 'Handlebars', group: 'Web / Markup', exts: ['hbs', 'handlebars'] },
        { value: 'htmlmixed', label: 'HTML', group: 'Web / Markup', exts: ['html', 'htm', 'xhtml', 'vue', 'svelte'] },
        { value: 'jinja2', label: 'Jinja2', group: 'Web / Markup', exts: ['j2', 'jinja', 'jinja2'] },
        { value: 'application/json', label: 'JSON', group: 'Web / Markup', exts: ['json', 'jsonc', 'json5', 'map'] },
        { value: 'text/x-less', label: 'LESS', group: 'Web / Markup', exts: ['less'] },
        { value: 'markdown', label: 'Markdown', group: 'Web / Markup', exts: ['md', 'markdown', 'mdown', 'mkd'] },
        { value: 'pug', label: 'Pug / Jade', group: 'Web / Markup', exts: ['pug', 'jade'] },
        { value: 'text/x-sass', label: 'Sass', group: 'Web / Markup', exts: ['sass'] },
        { value: 'text/x-scss', label: 'SCSS', group: 'Web / Markup', exts: ['scss'] },
        { value: 'stylus', label: 'Stylus', group: 'Web / Markup', exts: ['styl'] },
        { value: 'twig', label: 'Twig', group: 'Web / Markup', exts: ['twig'] },
        { value: 'xml', label: 'XML', group: 'Web / Markup', exts: ['xml', 'xsl', 'xslt', 'xsd', 'rss', 'svg', 'plist'] },

        // --- Scripting ---
        { value: 'coffeescript', label: 'CoffeeScript', group: 'Scripting', exts: ['coffee'] },
        { value: 'dart', label: 'Dart', group: 'Scripting', exts: ['dart'] },
        { value: 'groovy', label: 'Groovy', group: 'Scripting', exts: ['groovy', 'gradle'] },
        { value: 'julia', label: 'Julia', group: 'Scripting', exts: ['jl'] },
        { value: 'lua', label: 'Lua', group: 'Scripting', exts: ['lua'] },
        { value: 'perl', label: 'Perl', group: 'Scripting', exts: ['pl', 'pm', 'perl'] },
        { value: 'powershell', label: 'PowerShell', group: 'Scripting', exts: ['ps1', 'psm1', 'psd1'] },
        { value: 'r', label: 'R', group: 'Scripting', exts: ['r'] },
        { value: 'shell', label: 'Shell / Bash', group: 'Scripting', exts: ['sh', 'bash', 'zsh', 'ksh', 'bashrc', 'zshrc', 'profile'] },
        { value: 'tcl', label: 'Tcl', group: 'Scripting', exts: ['tcl'] },

        // --- Systems / Hardware ---
        { value: 'gas', label: 'Assembly (GAS)', group: 'Systems / Hardware', exts: ['s', 'asm'] },
        { value: 'cobol', label: 'COBOL', group: 'Systems / Hardware', exts: ['cob', 'cbl', 'cobol'] },
        { value: 'crystal', label: 'Crystal', group: 'Systems / Hardware', exts: ['cr'] },
        { value: 'd', label: 'D', group: 'Systems / Hardware', exts: ['d'] },
        { value: 'text/x-fortran', label: 'Fortran', group: 'Systems / Hardware', exts: ['f', 'for', 'f90', 'f95', 'f03'] },
        { value: 'haxe', label: 'Haxe', group: 'Systems / Hardware', exts: ['hx'] },
        { value: 'pascal', label: 'Pascal', group: 'Systems / Hardware', exts: ['pas', 'pp'] },
        { value: 'verilog', label: 'Verilog', group: 'Systems / Hardware', exts: ['v', 'sv', 'svh'] },
        { value: 'vhdl', label: 'VHDL', group: 'Systems / Hardware', exts: ['vhd', 'vhdl'] },
        { value: 'wast', label: 'WebAssembly', group: 'Systems / Hardware', exts: ['wat', 'wast'] },
        { value: 'z80', label: 'Z80 Assembly', group: 'Systems / Hardware', exts: ['z80'] },

        // --- Functional ---
        { value: 'clojure', label: 'Clojure', group: 'Functional', exts: ['clj', 'cljs', 'cljc', 'edn'] },
        { value: 'commonlisp', label: 'Common Lisp', group: 'Functional', exts: ['lisp', 'cl', 'el', 'lsp'] },
        { value: 'elm', label: 'Elm', group: 'Functional', exts: ['elm'] },
        { value: 'erlang', label: 'Erlang', group: 'Functional', exts: ['erl', 'hrl'] },
        { value: 'text/x-fsharp', label: 'F#', group: 'Functional', exts: ['fs', 'fsx', 'fsi'] },
        { value: 'haskell', label: 'Haskell', group: 'Functional', exts: ['hs', 'lhs'] },
        { value: 'text/x-ocaml', label: 'OCaml', group: 'Functional', exts: ['ml', 'mli'] },
        { value: 'scheme', label: 'Scheme', group: 'Functional', exts: ['scm', 'ss'] },
        { value: 'smalltalk', label: 'Smalltalk', group: 'Functional', exts: ['st'] },

        // --- Data / Config ---
        { value: 'cmake', label: 'CMake', group: 'Data / Config', exts: ['cmake'] },
        { value: 'diff', label: 'Diff / Patch', group: 'Data / Config', exts: ['diff', 'patch'] },
        { value: 'dockerfile', label: 'Dockerfile', group: 'Data / Config', exts: ['dockerfile'] },
        { value: 'properties', label: 'INI / Properties', group: 'Data / Config', exts: ['ini', 'cfg', 'conf', 'properties', 'env'] },
        { value: 'nginx', label: 'Nginx', group: 'Data / Config', exts: [] },
        { value: 'protobuf', label: 'Protocol Buffers', group: 'Data / Config', exts: ['proto'] },
        { value: 'sql', label: 'SQL', group: 'Data / Config', exts: ['sql'] },
        { value: 'toml', label: 'TOML', group: 'Data / Config', exts: ['toml'] },
        { value: 'yaml', label: 'YAML', group: 'Data / Config', exts: ['yml', 'yaml'] },

        // --- Scientific / Other ---
        { value: 'apl', label: 'APL', group: 'Scientific / Other', exts: ['apl'] },
        { value: 'brainfuck', label: 'Brainfuck', group: 'Scientific / Other', exts: ['bf'] },
        { value: 'gherkin', label: 'Gherkin', group: 'Scientific / Other', exts: ['feature'] },
        { value: 'stex', label: 'LaTeX', group: 'Scientific / Other', exts: ['tex', 'latex', 'sty', 'cls'] },
        { value: 'mathematica', label: 'Mathematica', group: 'Scientific / Other', exts: ['wl', 'wls'] },
        { value: 'text/x-octave', label: 'MATLAB / Octave', group: 'Scientific / Other', exts: ['matlab'] },
        { value: 'application/n-triples', label: 'N-Triples', group: 'Scientific / Other', exts: ['nt'] },
        { value: 'rst', label: 'reStructuredText', group: 'Scientific / Other', exts: ['rst'] },
        { value: 'sparql', label: 'SPARQL', group: 'Scientific / Other', exts: ['rq', 'sparql'] },
        { value: 'textile', label: 'Textile', group: 'Scientific / Other', exts: ['textile'] },
        { value: 'turtle', label: 'Turtle (RDF)', group: 'Scientific / Other', exts: ['ttl'] },
        { value: 'vbscript', label: 'VBScript', group: 'Scientific / Other', exts: ['vbs'] },
        { value: 'text/x-vb', label: 'Visual Basic', group: 'Scientific / Other', exts: ['vb'] },
    ];

    // Ordered list of vendored CodeMirror scripts to load. Order matters:
    // addons and base modes must load before the composite modes that build on
    // them (e.g. htmlmixed needs xml/javascript/css; php needs htmlmixed+clike).
    const MODE_SCRIPTS = [
        // mode-loading addons
        'vendor/codemirror/addon/mode/simple.js',
        'vendor/codemirror/addon/mode/multiplex.js',
        'vendor/codemirror/addon/mode/overlay.js',
        // base modes (no cross-mode dependencies)
        'vendor/codemirror/mode/xml/xml.min.js',
        'vendor/codemirror/mode/javascript/javascript.min.js',
        'vendor/codemirror/mode/css/css.min.js',
        'vendor/codemirror/mode/clike/clike.min.js',
        'vendor/codemirror/mode/sql/sql.min.js',
        'vendor/codemirror/mode/markdown/markdown.min.js',
        'vendor/codemirror/mode/yaml/yaml.min.js',
        'vendor/codemirror/mode/shell/shell.min.js',
        'vendor/codemirror/mode/python/python.min.js',
        'vendor/codemirror/mode/perl/perl.min.js',
        'vendor/codemirror/mode/lua/lua.min.js',
        // standalone modes
        'vendor/codemirror/mode/rust/rust.min.js',
        'vendor/codemirror/mode/go/go.min.js',
        'vendor/codemirror/mode/ruby/ruby.min.js',
        'vendor/codemirror/mode/swift/swift.min.js',
        'vendor/codemirror/mode/dart/dart.min.js',
        'vendor/codemirror/mode/r/r.min.js',
        'vendor/codemirror/mode/julia/julia.min.js',
        'vendor/codemirror/mode/groovy/groovy.min.js',
        'vendor/codemirror/mode/coffeescript/coffeescript.min.js',
        'vendor/codemirror/mode/haskell/haskell.min.js',
        'vendor/codemirror/mode/mllike/mllike.min.js',
        'vendor/codemirror/mode/elm/elm.min.js',
        'vendor/codemirror/mode/erlang/erlang.min.js',
        'vendor/codemirror/mode/clojure/clojure.min.js',
        'vendor/codemirror/mode/commonlisp/commonlisp.min.js',
        'vendor/codemirror/mode/scheme/scheme.min.js',
        'vendor/codemirror/mode/smalltalk/smalltalk.min.js',
        'vendor/codemirror/mode/tcl/tcl.min.js',
        'vendor/codemirror/mode/cmake/cmake.min.js',
        'vendor/codemirror/mode/dockerfile/dockerfile.min.js',
        'vendor/codemirror/mode/nginx/nginx.min.js',
        'vendor/codemirror/mode/toml/toml.min.js',
        'vendor/codemirror/mode/properties/properties.min.js',
        'vendor/codemirror/mode/diff/diff.min.js',
        'vendor/codemirror/mode/protobuf/protobuf.min.js',
        'vendor/codemirror/mode/gherkin/gherkin.min.js',
        'vendor/codemirror/mode/cobol/cobol.min.js',
        'vendor/codemirror/mode/fortran/fortran.min.js',
        'vendor/codemirror/mode/pascal/pascal.min.js',
        'vendor/codemirror/mode/d/d.min.js',
        'vendor/codemirror/mode/crystal/crystal.min.js',
        'vendor/codemirror/mode/haxe/haxe.min.js',
        'vendor/codemirror/mode/verilog/verilog.min.js',
        'vendor/codemirror/mode/vhdl/vhdl.min.js',
        'vendor/codemirror/mode/wast/wast.min.js',
        'vendor/codemirror/mode/gas/gas.min.js',
        'vendor/codemirror/mode/z80/z80.min.js',
        'vendor/codemirror/mode/octave/octave.min.js',
        'vendor/codemirror/mode/vb/vb.min.js',
        'vendor/codemirror/mode/vbscript/vbscript.min.js',
        'vendor/codemirror/mode/powershell/powershell.min.js',
        'vendor/codemirror/mode/apl/apl.min.js',
        'vendor/codemirror/mode/brainfuck/brainfuck.min.js',
        'vendor/codemirror/mode/mathematica/mathematica.min.js',
        'vendor/codemirror/mode/rst/rst.min.js',
        'vendor/codemirror/mode/stex/stex.min.js',
        'vendor/codemirror/mode/textile/textile.min.js',
        'vendor/codemirror/mode/sass/sass.min.js',
        'vendor/codemirror/mode/stylus/stylus.min.js',
        'vendor/codemirror/mode/ntriples/ntriples.min.js',
        'vendor/codemirror/mode/turtle/turtle.min.js',
        'vendor/codemirror/mode/sparql/sparql.min.js',
        'vendor/codemirror/mode/jinja2/jinja2.min.js',
        'vendor/codemirror/mode/django/django.min.js',
        // composite modes (depend on base modes above)
        'vendor/codemirror/mode/htmlmixed/htmlmixed.min.js',
        'vendor/codemirror/mode/jsx/jsx.min.js',
        'vendor/codemirror/mode/php/php.min.js',
        'vendor/codemirror/mode/pug/pug.min.js',
        'vendor/codemirror/mode/handlebars/handlebars.min.js',
        'vendor/codemirror/mode/twig/twig.min.js',
    ];

    // value -> human label, derived from LANGUAGES.
    const LANG_LABELS = {};
    for (const lang of LANGUAGES) LANG_LABELS[lang.value] = lang.label;

    // extension (lower-case, no dot) -> language value, derived from LANGUAGES.
    // First definition wins so the table's ordering resolves any ambiguity.
    const EXT_MODE = {};
    for (const lang of LANGUAGES) {
        for (const ext of lang.exts) {
            if (!(ext in EXT_MODE)) EXT_MODE[ext] = lang.value;
        }
    }

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

    // Map a full path/filename to a language value. Tries the extension first,
    // then falls back to well-known extension-less filenames (e.g. Dockerfile).
    function modeForFilename(path) {
        if (!path) return 'plaintext';
        const name = String(path).split(/[/\\]/).pop();
        if (name.indexOf('.') === -1) {
            return EXT_MODE[name.toLowerCase()] || 'plaintext';
        }
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

    // Escape a literal string so it can be embedded in a RegExp.
    function escapeRegExp(text) {
        return String(text).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    }

    // Build the RegExp used by the Find/Replace popup. Returns null for empty
    // input or an invalid user-supplied pattern (so callers can no-op safely).
    function buildSearchQuery(text, opts) {
        opts = opts || {};
        if (!text) return null;
        const flags = opts.caseSensitive ? '' : 'i';
        const source = opts.regex ? String(text) : escapeRegExp(text);
        try {
            return new RegExp(source, flags);
        } catch (e) {
            return null;
        }
    }

    // Clamp a 1-based line number to a document of `lineCount` lines.
    function clampLine(n, lineCount) {
        const v = parseInt(n, 10);
        if (!Number.isFinite(v)) return null;
        return Math.max(1, Math.min(lineCount, v));
    }

    return {
        LANGUAGES, MODE_SCRIPTS, LANG_LABELS, EXT_MODE,
        modeLabel, resolveMode, extToMode, modeForFilename, basename, detectEol, eolJoin,
        escapeRegExp, buildSearchQuery, clampLine,
    };
});
