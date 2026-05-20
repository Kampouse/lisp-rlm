<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as monaco from 'monaco-editor';
  import { initCompiler, compile, runPure, runNear, compileP2Core, toHexDump, getNearStorage, clearNearStorage, getNearContext, setNearContext, resetNearContext, decodeReturnValue, formatGas, type CompileTarget, type CompileResult, type NearContext } from './lib/compiler.ts';
  import { runWasiWithWorker } from './lib/runWasiWithWorker.ts';
  import { examples } from './lib/examples.ts';
  import { connectWallet, disconnectWallet, deployP1, deployP2, getWalletState, type WalletState, type DeployResult, type Network } from './lib/wallet.ts';
  import { parseTests, buildTestCode, type TestRunResult } from './lib/test-runner.ts';
  import { Play, Box, Cloud, Zap, Link, FlaskConical, Wallet, Rocket, CircleDot, Loader2, ChevronDown, ChevronUp, Menu, X, BookOpen, CheckCircle, XCircle, Hammer, Database, Trash2, FolderOpen, FileCode, ChevronRight } from '@lucide/svelte';

  // ============================================
  // Code Outline
  // ============================================
  interface OutlineItem {
    kind: 'function' | 'variable' | 'test' | 'define';
    name: string;
    line: number;
  }

  function parseOutline(src: string): OutlineItem[] {
    const items: OutlineItem[] = [];
    const lines = src.split('\n');
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      // (defun name ...)
      const defunMatch = line.match(/^\(\s*defun\s+([^\s(]+)/);
      if (defunMatch) { items.push({ kind: 'function', name: defunMatch[1], line: i + 1 }); continue; }
      // (define (name ...) ...)
      const defineMatch = line.match(/^\(\s*define\s+\(\s*([^\s)]+)/);
      if (defineMatch) { items.push({ kind: 'define', name: defineMatch[1], line: i + 1 }); continue; }
      // (defvar name ...)
      const defvarMatch = line.match(/^\(\s*defvar\s+([^\s)]+)/);
      if (defvarMatch) { items.push({ kind: 'variable', name: defvarMatch[1], line: i + 1 }); continue; }
      // (test "name" ...)
      const testMatch = line.match(/^\(\s*test\s+["']([^"']+)["']/);
      if (testMatch) { items.push({ kind: 'test', name: testMatch[1], line: i + 1 }); continue; }
    }
    return items;
  }

  function outlineIcon(kind: OutlineItem['kind']): string {
    switch (kind) {
      case 'function': return 'ƒ';
      case 'define': return 'ƒ';
      case 'variable': return '×';
      case 'test': return '✓';
    }
  }

  function jumpToLine(line: number) {
    if (!editorInstance) return;
    editorInstance.revealLineInCenter(line);
    editorInstance.setPosition({ lineNumber: line, column: 1 });
    editorInstance.focus();
  }

  let showOutline: boolean = $state(true);
  let outlineItems: OutlineItem[] = $derived(parseOutline(source));

  // ============================================
  // API Reference (per target)
  // ============================================
  let showApiRef: boolean = $state(false);
  let apiExpanded: Record<string, boolean> = $state({});

  interface ApiGroup { title: string; items: string[]; }

  const CORE_MATH: string[] = ['+', '-', '*', '/', '%', 'mod', 'abs', 'min', 'max', 'inc', 'dec'];
  const CORE_CMP: string[] = ['=', '!=', '<', '>', '<=', '>=', 'zero?', 'pos?', 'neg?', 'even?', 'odd?'];
  const CORE_LOGIC: string[] = ['and', 'or', 'not', 'if', 'cond', 'case', 'when', 'unless', 'match'];
  const CORE_LIST: string[] = ['list', 'car', 'cdr', 'cons', 'append', 'nth', 'length', 'reverse', 'sort', 'range', 'map', 'filter', 'reduce', 'find', 'member', 'take', 'drop', 'zip'];
  const CORE_STR: string[] = ['str-concat', 'str-length', 'str-substring', 'str-split', 'str-index-of', 'str-contains', 'str-replace', 'to-string', 'str->num', 'num->str'];
  const CORE_PRED: string[] = ['nil?', 'list?', 'number?', 'string?', 'bool?', 'symbol?', 'procedure?', 'type-of', 'equal?'];
  const CORE_FORM: string[] = ['define', 'defn', 'def', 'let', 'fn', 'lambda', 'loop', 'recur', 'set!', 'quote', 'do', 'begin'];
  const CORE_DICT: string[] = ['dict', 'dict/get', 'dict/set', 'dict/has?', 'dict/keys', 'dict/vals', 'dict/remove', 'dict-merge'];
  const CORE_JSON: string[] = ['json-parse', 'to-json', 'from-json', 'json-get', 'json/get'];
  const CORE_VEC: string[] = ['vec', 'vec?', 'vec-nth', 'vec-len', 'vec-conj', 'vec-slice'];
  const CORE_BORSH: string[] = ['borsh-serialize', 'borsh-deserialize', 'array'];
  const CORE_FP: string[] = ['fp/mul', 'fp/div', 'fp/to_int', 'fp/from_int', 'fp/sqrt'];
  const CORE_U128: string[] = ['u128/new', 'u128/store', 'u128/load', 'u128/add', 'u128/sub', 'u128/mul', 'u128/lt', 'u128/eq'];
  const CORE_PRINT: string[] = ['print', 'println', 'display', 'newline', 'debug', 'error'];
  const CORE_TEST: string[] = ['test', 'assert-equal', 'assert-true', 'assert-false', 'assert-returns'];

  const NEAR_STORAGE: string[] = ['near/store', 'near/load', 'near/remove', 'near/has_key', 'near/iter_prefix', 'near/iter_next'];
  const NEAR_CTX: string[] = ['near/current_account_id', 'near/signer_account_id', 'near/predecessor_account_id', 'near/input', 'near/block_index', 'near/block_timestamp', 'near/epoch_height'];
  const NEAR_CRYPTO: string[] = ['near/sha256', 'near/keccak256', 'near/ed25519_verify', 'near/ecrecover', 'near/p256_verify', 'near/random_seed'];
  const NEAR_BALANCE: string[] = ['near/account_balance', 'near/attached_deposit', 'near/prepaid_gas', 'near/used_gas', 'near/storage_usage'];
  const NEAR_PROMISE: string[] = ['near/promise_create', 'near/promise_then', 'near/promise_and', 'near/promise_return', 'near/promise_results_count', 'near/promise_result'];
  const NEAR_JSON: string[] = ['near/json_get_int', 'near/json_get_str', 'near/json_return_int', 'near/json_return_str'];
  const NEAR_BATCH: string[] = ['near/promise_batch_create', 'near/promise_batch_then', 'near/promise_batch_action_create_account', 'near/promise_batch_action_deploy_contract', 'near/promise_batch_action_function_call', 'near/promise_batch_action_transfer', 'near/promise_batch_action_stake', 'near/promise_batch_action_add_key_with_full_access', 'near/promise_batch_action_delete_key', 'near/promise_batch_action_delete_account'];
  const NEAR_MISC: string[] = ['near/log', 'near/panic', 'near/return', 'near/abort', 'near/validator_stake', 'near/validator_total_stake'];

  const WASI_STORAGE: string[] = ['storage-set', 'storage-get', 'storage-has', 'storage-delete', 'storage-increment', 'storage-decrement', 'storage-list-keys'];
  const WASI_ENV: string[] = ['env/signer', 'env/predecessor'];
  const WASI_HTTP: string[] = ['http-get', 'http-post', 'http-get-json'];
  const WASI_OUTLAYER: string[] = ['outlayer/view', 'outlayer/raw', 'outlayer/status', 'outlayer/context', 'outlayer/storage-set', 'outlayer/storage-get', 'outlayer/storage-has', 'outlayer/storage-delete', 'outlayer/call', 'outlayer/transfer', 'outlayer/http_get'];

  function getApiForTarget(t: CompileTarget): ApiGroup[] {
    const core: ApiGroup[] = [
      { title: 'Forms', items: CORE_FORM },
      { title: 'Math', items: CORE_MATH },
      { title: 'Compare', items: CORE_CMP },
      { title: 'Logic', items: CORE_LOGIC },
      { title: 'List', items: CORE_LIST },
      { title: 'String', items: CORE_STR },
      { title: 'Dict', items: CORE_DICT },
      { title: 'JSON', items: CORE_JSON },
      { title: 'Vec', items: CORE_VEC },
      { title: 'Predicate', items: CORE_PRED },
      { title: 'Borsh', items: CORE_BORSH },
      { title: 'FixedPoint', items: CORE_FP },
      { title: 'u128', items: CORE_U128 },
      { title: 'Print', items: CORE_PRINT },
      { title: 'Test', items: CORE_TEST },
    ];

    if (t === 'p1') {
      core.push(
        { title: 'NEAR Storage', items: NEAR_STORAGE },
        { title: 'NEAR Context', items: NEAR_CTX },
        { title: 'NEAR Crypto', items: NEAR_CRYPTO },
        { title: 'NEAR Balance', items: NEAR_BALANCE },
        { title: 'NEAR JSON', items: NEAR_JSON },
        { title: 'NEAR Promise', items: NEAR_PROMISE },
        { title: 'NEAR Batch', items: NEAR_BATCH },
        { title: 'NEAR Misc', items: NEAR_MISC },
      );
    } else if (t === 'p2') {
      core.push(
        { title: 'Storage', items: WASI_STORAGE },
        { title: 'Env', items: WASI_ENV },
        { title: 'HTTP', items: WASI_HTTP },
        { title: 'OutLayer', items: WASI_OUTLAYER },
      );
    }
    return core;
  }

  let apiForTarget: ApiGroup[] = $derived(getApiForTarget(target));

  function insertSnippet(fn: string) {
    if (!editorInstance) return;
    const pos = editorInstance.getPosition();
    editorInstance.executeEdits('api-ref', [{
      range: new monaco.Range(pos.lineNumber, pos.column, pos.lineNumber, pos.column),
      text: `(${fn} )`,
    }]);
    editorInstance.setPosition({ lineNumber: pos.lineNumber, column: pos.column + fn.length + 2 });
    editorInstance.focus();
  }

  // ============================================
  // State
  // ============================================
  let target: CompileTarget = $state('pure');
  let source: string = $state('');
  let wasmReady: boolean = $state(false);
  let compiling: boolean = $state(false);
  let deploying: boolean = $state(false);
  let result: CompileResult | null = $state(null);
  let deployResult: DeployResult | null = $state(null);
  let walletState: WalletState = $state({ connected: false, accountId: null, network: 'mainnet' });
  let activeExample: number = $state(0);
  let editorInstance: monaco.editor.IStandaloneCodeEditor | null = $state(null);
  let editorContainer: HTMLDivElement | null = $state(null);
  let contractName: string = $state('my-contract');
  let network: Network = $state('mainnet');
  let showDeployPanel: boolean = $state(false);
  let runResult: string | null = $state(null);
  let running: boolean = $state(false);
  let showWat: boolean = $state(false);

  // Feature 5: Auto-compile toggle
  let autoCompile: boolean = $state(true);
  let compileDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Feature 9: REPL mode
  let replMode: boolean = $state(false);
  let replHistory: { expr: string; result: string }[] = $state([]);
  let replInput: string = $state('');

  // Mobile examples menu
  let showExamplesMenu: boolean = $state(false);

  // Learn panel
  let showLearn: boolean = $state(false);

  // Test runner
  let testResults: TestRunResult | null = $state(null);
  let testing: boolean = $state(false);

  // NEAR storage inspector
  let nearStorageView: Record<string, string> | null = $state(null);
  let showNearStorage: boolean = $state(false);

  // NEAR method runner
  let nearMethods: string[] = $state([]);
  let selectedMethod: string = $state('');
  let nearGasUsed: string = $state('');
  let nearReturnDisplay: string | null = $state(null);
  let showNearContext: boolean = $state(false);
  let nearCtx: NearContext = $state(getNearContext());

  // NEAR run results (new fields)
  let nearInputJson: string = $state('');
  let nearLogs: string[] = $state([]);
  let nearPanic: string | null = $state(null);
  let nearStorageDiff: Array<{ key: string; oldVal: string | null; newVal: string | null }> = $state([]);
  let nearReceipts: Array<{ index: number; accountId: string; methodName: string; argsSize: number; result?: Uint8Array; type: string }> = $state([]);

  // Resizable panes
  let outputPaneWidth: number = $state(40); // percentage
  let isResizing: boolean = $state(false);
  let outputCollapsed: boolean = $state(false);

  function startResize(e: MouseEvent) {
    isResizing = true;
    document.addEventListener('mousemove', handleResize);
    document.addEventListener('mouseup', stopResize);
  }

  function handleResize(e: MouseEvent) {
    if (!isResizing) return;
    const container = document.querySelector('.split-container');
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const newWidth = ((containerRect.right - e.clientX) / containerRect.width) * 100;
    outputPaneWidth = Math.max(20, Math.min(70, newWidth));
  }

  function stopResize() {
    isResizing = false;
    document.removeEventListener('mousemove', handleResize);
    document.removeEventListener('mouseup', stopResize);
  }

  // Derived
  let hexDump: string = $derived.by(() => {
    if (result && result.success && result.wasmBytes) {
      return toHexDump(result.wasmBytes);
    }
    return '';
  });

  let shortAccountId: string = $derived.by(() => {
    if (!walletState.accountId) return '';
    const id = walletState.accountId;
    if (id.length > 20) return id.slice(0, 6) + '…' + id.slice(-8);
    return id;
  });

  // ============================================
  // Feature 8: localStorage persistence
  // ============================================
  const STORAGE_KEY = 'lisp-rlm-state';
  const FILES_KEY = 'lisp-rlm-files';

  // ============================================
  // Virtual File System
  // ============================================
  interface VFile {
    id: string;
    name: string;
    source: string;
    target: CompileTarget;
    updatedAt: number;
  }

  let files: VFile[] = $state([]);
  let activeFileId: string = $state('');
  let renamingFileId: string | null = $state(null);
  let renameInput: string = $state('');
  let fileContextMenu: { fileId: string; x: number; y: number } | null = $state(null);

  function generateId(): string {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
  }

  function loadFiles(): VFile[] {
    try {
      const stored = localStorage.getItem(FILES_KEY);
      if (stored) return JSON.parse(stored);
    } catch {}
    return [];
  }

  function saveFiles() {
    try {
      localStorage.setItem(FILES_KEY, JSON.stringify(files));
    } catch {}
  }

  function saveCurrentFile() {
    if (!activeFileId) return;
    const idx = files.findIndex(f => f.id === activeFileId);
    if (idx >= 0) {
      files[idx].source = source;
      files[idx].target = target;
      files[idx].updatedAt = Date.now();
      files = files; // trigger reactivity
      saveFiles();
    }
  }

  function createFile(name?: string) {
    const fname = name || 'untitled.lisp';
    const file: VFile = { id: generateId(), name: fname, source: '', target: 'pure', updatedAt: Date.now() };
    files = [file, ...files];
    switchToFile(file.id);
    saveFiles();
  }

  function deleteFile(id: string) {
    const idx = files.findIndex(f => f.id === id);
    if (idx < 0) return;
    files = files.filter(f => f.id !== id);
    if (id === activeFileId) {
      if (files.length > 0) {
        switchToFile(files[0].id);
      } else {
        createFile('main.lisp');
      }
    }
    saveFiles();
    fileContextMenu = null;
  }

  function duplicateFile(id: string) {
    const src = files.find(f => f.id === id);
    if (!src) return;
    const file: VFile = { id: generateId(), name: src.name.replace('.lisp', '-copy.lisp'), source: src.source, target: src.target, updatedAt: Date.now() };
    files = [file, ...files];
    switchToFile(file.id);
    saveFiles();
    fileContextMenu = null;
  }

  function startRename(id: string) {
    const f = files.find(f => f.id === id);
    if (!f) return;
    renamingFileId = id;
    renameInput = f.name.replace('.lisp', '');
    fileContextMenu = null;
  }

  function commitRename() {
    if (!renamingFileId) return;
    const f = files.find(f => f.id === renamingFileId);
    if (f) {
      f.name = renameInput.endsWith('.lisp') ? renameInput : renameInput + '.lisp';
      files = files;
      saveFiles();
    }
    renamingFileId = null;
    renameInput = '';
  }

  function switchToFile(id: string) {
    // Save current file first
    saveCurrentFile();
    const f = files.find(f => f.id === id);
    if (!f) return;
    activeFileId = id;
    source = f.source;
    target = f.target;
    if (editorInstance) editorInstance.setValue(source);
    result = null;
    runResult = null;
    testResults = null;
    deployResult = null;
    showDeployPanel = false;
    clearMonacoMarkers();
    saveState();
  }

  function handleFileContextMenu(e: MouseEvent, fileId: string) {
    e.preventDefault();
    fileContextMenu = { fileId, x: e.clientX, y: e.clientY };
  }

  function closeContextMenu() {
    fileContextMenu = null;
  }

  function activeFileName(): string {
    return files.find(f => f.id === activeFileId)?.name || 'untitled.lisp';
  }

  function saveState() {
    try {
      const state = { source, target, activeExample, autoCompile, replMode, activeFileId };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (e) {
      console.warn('Failed to save state:', e);
    }
  }

  function loadState(): { source: string; target: CompileTarget; activeExample: number; autoCompile: boolean; replMode: boolean; activeFileId?: string } | null {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        return {
          source: parsed.source || examples[0].source,
          target: parsed.target || 'pure',
          activeExample: parsed.activeExample ?? 0,
          autoCompile: parsed.autoCompile ?? true,
          replMode: parsed.replMode ?? false,
          activeFileId: parsed.activeFileId,
        };
      }
    } catch (e) {
      console.warn('Failed to load state:', e);
    }
    return null;
  }

  // ============================================
  // Feature 7: Shareable URLs
  // ============================================
  function updateUrlHash() {
    try {
      const params = new URLSearchParams();
      if (source && source !== examples[activeExample]?.source) {
        // Custom source - encode it
        params.set('code', btoa(encodeURIComponent(source)));
      }
      params.set('target', target);
      if (activeExample > 0) params.set('example', String(activeExample));
      const hash = '#' + params.toString();
      if (history.replaceState) {
        history.replaceState(null, '', hash);
      }
    } catch (e) {
      // Ignore URL errors
    }
  }

  function loadFromUrl(): { source?: string; target?: CompileTarget; example?: number } {
    try {
      const hash = window.location.hash.slice(1);
      if (!hash) return {};
      const params = new URLSearchParams(hash);
      const result: { source?: string; target?: CompileTarget; example?: number } = {};
      if (params.has('code')) {
        result.source = decodeURIComponent(atob(params.get('code')!));
      }
      if (params.has('target')) {
        result.target = params.get('target') as CompileTarget;
      }
      if (params.has('example')) {
        result.example = parseInt(params.get('example')!, 10);
      }
      return result;
    } catch (e) {
      return {};
    }
  }

  // ============================================
  // Feature 4: Monaco error markers
  // ============================================
  function showMonacoMarkers(errors: monaco.editor.IMarkerData[]) {
    const model = editorInstance?.getModel();
    if (model) {
      // No setModelMarkers — Error Lens handles display (line bg + inline text)
      showErrorLens(errors);
    }
  }

  function clearMonacoMarkers() {
    clearErrorLens();
  }

  // Error Lens — inline error messages at end of line (like VS Code Error Lens extension)
  let errorLensDecorations: string[] = [];
  let errorLensOverlay: HTMLDivElement | null = null;

  function showErrorLens(markers: monaco.editor.IMarkerData[]) {
    if (!editorInstance) return;
    clearErrorLens();

    // Whole-line background highlight + squiggly underline already from markers
    const newDecorations: monaco.editor.IModelDeltaDecoration[] = markers.map(m => ({
      range: new monaco.Range(m.startLineNumber, 1, m.startLineNumber, 1),
      options: {
        isWholeLine: true,
        className: m.severity === monaco.MarkerSeverity.Error ? 'error-lens-line-error' : 'error-lens-line-warning',
        overviewRuler: {
          color: m.severity === monaco.MarkerSeverity.Error ? '#ff4444' : '#ffaa00',
          position: monaco.editor.OverviewRulerLane.Full
        },
        // Inline text after the line content (line-decoration, not whole-line)
        afterContentClassName: m.severity === monaco.MarkerSeverity.Error ? 'error-lens-after-error' : 'error-lens-after-warning',
      }
    }));
    errorLensDecorations = editorInstance.deltaDecorations(errorLensDecorations, newDecorations);

    // Inject CSS with the error messages as ::after content
    const existing = document.getElementById('error-lens-styles');
    if (existing) existing.remove();
    const style = document.createElement('style');
    style.id = 'error-lens-styles';
    let css = '';
    for (let i = 0; i < markers.length; i++) {
      const m = markers[i];
      const isError = m.severity === monaco.MarkerSeverity.Error;
      const msg = m.message.length > 80 ? m.message.slice(0, 77) + '...' : m.message;
      const escapedMsg = msg.replace(/'/g, "\\'").replace(/"/g, '\\"');
      css += `.error-lens-${isError ? 'after-error' : 'after-warning'}::after { content: '  ${isError ? '✕' : '⚠'} ${escapedMsg}'; color: ${isError ? '#ff6b6b' : '#ffaa44'}; font-style: italic; font-size: 12px; opacity: 0.9; }\n`;
    }
    style.textContent = css;
    document.head.appendChild(style);
  }

  function clearErrorLens() {
    if (editorInstance) {
      errorLensDecorations = editorInstance.deltaDecorations(errorLensDecorations, []);
    }
    const style = document.getElementById('error-lens-styles');
    if (style) style.remove();
  }

  // ============================================
  // Monaco setup
  // ============================================
  function setupMonaco() {
    if (!editorContainer) return;

    // Use Monaco's built-in Clojure tokenizer — same Lisp family,
    // handles defn, defmacro, let, if, cond, ns, require, etc.
    // Extend with our lisp-rlm–specific keywords
    monaco.languages.register({ id: 'lisp-rlm' });
    
    monaco.languages.setMonarchTokensProvider('lisp-rlm', {
      ignoreCase: true,
      brackets: [
        { open: '(', close: ')', token: 'delimiter.parenthesis' },
        { open: '[', close: ']', token: 'delimiter.square' },
        { open: '{', close: '}', token: 'delimiter.bracket' },
      ],
      keywords: [
        // Core Clojure forms
        'def', 'defn', 'defn-', 'defmacro', 'defonce', 'defmethod',
        'fn', 'lambda', 'let', 'let*', 'loop', 'recur',
        'if', 'if-not', 'if-let', 'if-some', 'when', 'when-not', 'when-let', 'when-some',
        'cond', 'condp', 'case', 'do', 'doseq', 'dotimes', 'while',
        'and', 'or', 'not',
        'true', 'false', 'nil',
        // Lisp-RLM specific
        'define', 'defun', 'defvar', 'set!',
        'test', 'assert-equal', 'assert',
        'begin',
        // NEAR host functions
        'near/log', 'near/storage-read', 'near/storage-write',
        'near/value-return', 'near/input', 'near/account-id',
        'near/block-index', 'near/block-timestamp', 'near/storage-usage',
        'near/balance', 'near/attached-deposit', 'near/prepaid-gas',
        'near/used-gas', 'near/signer-account-id', 'near/signer-account-pk',
        'near/panic', 'near/panic-utf8',
        // WASI
        'wasi/args-get', 'wasi/environ-get', 'wasi/fd-write',
      ],
      constants: ['true', 'false', 'nil', 'null'],
      operators: ['=', 'not=', '+', '-', '*', '/', '<', '>', '<=', '>=', '=='],
      // Characters valid in symbol names (includes ?, !, -, *, /)
      identifierPrefix: /[*!?+\-<>=/.a-zA-Z_]/,
      tokenizer: {
        root: [
          // Comments: ;; and ;
          { regex: ';.*$', action: { token: 'comment' } },
          // Strings
          { regex: '"', action: { token: 'string', next: '@string' } },
          // Numbers
          { regex: '0x[0-9a-fA-F]+', action: { token: 'number.hex' } },
          { regex: '-?[0-9]+\\.?[0-9]*', action: { token: 'number' } },
          // Keywords (:keyword)
          { regex: ':[a-zA-Z_*\\-!?+<>=/.][a-zA-Z0-9_*\\-!?+<>=/.]*', action: { token: 'tag' } },
          // Delimiters
          { regex: '[()\\[\\]{}]', action: { token: 'delimiter.parenthesis' } },
          // Special form at start of s-expression — keyword highlight
          {
            regex: '\\([ \\t]*([a-zA-Z_*\\-!?+<>=/.][a-zA-Z0-9_*\\-!?+<>=/.]*)',
            action: {
              cases: {
                '@keywords': { token: 'keyword' },
                '@default': { token: 'identifier' },
              },
            },
          },
          // Identifiers (allows ?, !, -, *, / in names — Clojure-style)
          {
            regex: '[a-zA-Z_*\\-!?+<>=/.][a-zA-Z0-9_*\\-!?+<>=/.]*',
            action: {
              cases: {
                '@keywords': { token: 'keyword' },
                '@operators': { token: 'operator' },
                '@constants': { token: 'constant' },
                '@default': { token: 'identifier' },
              },
            },
          },
          // Whitespace
          { regex: '\\s+', action: { token: 'white' } },
        ],
        string: [
          { regex: '"', action: { token: 'string', next: '@pop' } },
          { regex: '\\\\.', action: { token: 'string.escape' } },
          { regex: '[^"\\\\]+', action: { token: 'string' } },
        ],
      },
    });

    // Auto-matching for (, [, {
    monaco.languages.setLanguageConfiguration('lisp-rlm', {
      comments: {
        lineComment: ';',
      },
      brackets: [
        ['(', ')'],
        ['[', ']'],
        ['{', '}'],
      ],
      autoClosingPairs: [
        { open: '(', close: ')' },
        { open: '[', close: ']' },
        { open: '{', close: '}' },
        { open: '"', close: '"' },
      ],
      surroundingPairs: [
        { open: '(', close: ')' },
        { open: '[', close: ']' },
        { open: '{', close: '}' },
        { open: '"', close: '"' },
      ],
      indentationRules: {
        increaseIndentPattern: /[(\[{]\s*$/,
        decreaseIndentPattern: /^\s*[)\]}]/,
      },
      wordPattern: /[*!?+\-<>=/.a-zA-Z_][*!?+\-<>=/.a-zA-Z0-9_]*/,
    });

    monaco.editor.defineTheme('lisp-dark', {
      base: 'vs-dark',
      inherit: true,
      rules: [
        { token: 'comment', foreground: '555580', fontStyle: 'italic' },
        { token: 'keyword', foreground: 'ff8c00', fontStyle: 'bold' },
        { token: 'string', foreground: '7ec699' },
        { token: 'string.escape', foreground: 'f07178' },
        { token: 'number', foreground: 'f29e74' },
        { token: 'number.hex', foreground: 'f29e74' },
        { token: 'tag', foreground: 'ffcb6b' }, // :keywords
        { token: 'constant', foreground: '82aaff' },
        { token: 'identifier', foreground: 'c2c2d6' },
        { token: 'delimiter.parenthesis', foreground: '89ddff' },
        { token: 'operator', foreground: 'c792ea' },
      ],
      colors: {
        'editor.background': '#0f0f18',
        'editor.foreground': '#c2c2d6',
        'editor.lineHighlightBackground': '#16162a',
        'editor.selectionBackground': '#ff8c0033',
        'editorCursor.foreground': '#ff8c00',
        'editorLineNumber.foreground': '#333350',
        'editorLineNumber.activeForeground': '#555580',
        'editor.selectionHighlightBackground': '#ff8c0015',
        'editorIndentGuide.background': '#1a1a30',
        'editorIndentGuide.activeBackground': '#252545',
        'editorBracketMatch.background': '#ff8c0018',
        'editorBracketMatch.border': '#ff8c0055',
      },
    });

    editorInstance = monaco.editor.create(editorContainer, {
      value: source,
      language: 'lisp-rlm',
      theme: 'lisp-dark',
      fontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', Consolas, monospace",
      fontSize: 14,
      lineHeight: 22,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      padding: { top: 16, bottom: 16 },
      renderLineHighlight: 'gutter',
      smoothScrolling: true,
      cursorBlinking: 'smooth',
      cursorSmoothCaretAnimation: 'on',
      bracketPairColorization: { enabled: true },
      automaticLayout: true,
      tabSize: 2,
      wordWrap: 'on',
      scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: true,
      renderWhitespace: 'none',
      guides: { bracketPairs: true, indentation: true },
    });

    // Track content changes
    editorInstance.onDidChangeModelContent(() => {
      source = editorInstance?.getValue() ?? '';
      // Feature 6: Live recompile (debounced)
      if (autoCompile && !replMode) {
        scheduleCompile();
      }
      // Feature 7: Update URL
      updateUrlHash();
      // Feature 8: Save to localStorage
      saveState();
      saveCurrentFile();
    });
  }

  // ============================================
  // Compilation
  // ============================================
  
  // Feature 6: Debounced compile
  function scheduleCompile() {
    if (compileDebounceTimer) {
      clearTimeout(compileDebounceTimer);
    }
    compileDebounceTimer = setTimeout(() => {
      handleCompile(true); // auto = true
    }, 300);
  }

  async function handleCompile(auto: boolean = false) {
    if (!wasmReady || compiling) return;
    compiling = true;
    // Don't clear result — prevents output panel from collapsing during recompile
    deployResult = null;
    showDeployPanel = false;
    runResult = null;
    testResults = null;
    clearMonacoMarkers();
    await new Promise(r => setTimeout(r, 50));
    try {
      result = compile(source, target);
      
      // Populate NEAR methods list from compiled exports
      if (result.success && target === 'p1' && result.exports) {
        nearMethods = result.exports.filter(e => e !== '_run');
        if (nearMethods.length > 0 && !nearMethods.includes(selectedMethod)) {
          selectedMethod = '';
        }
      }
      
      // Feature 4: Show errors inline in Monaco
      if (!result.success && result.error) {
        const markers = parseErrorToMarkers(result.error);
        if (markers.length > 0) {
          showMonacoMarkers(markers);
        }
      }
      
      // Feature 1: Auto-run on compile for pure target (always, not just on debounce)
      if (result.success && result.wasmBytes && target === 'pure') {
        await handleRun();
      }

      // Auto-run tests on successful compile when source has tests
      if (result.success && auto && hasTests(source)) {
        handleRunTests();
      }
    } finally {
      compiling = false;
    }
  }

  // Feature 4: Parse error to markers
  function parseErrorToMarkers(error: string): monaco.editor.IMarkerData[] {
    // Try to extract line info from error
    const lineMatch = error.match(/line (\d+)/i);
    const line = lineMatch ? parseInt(lineMatch[1], 10) : 1;
    return [{
      severity: monaco.MarkerSeverity.Error,
      message: error,
      startLineNumber: line,
      startColumn: 1,
      endLineNumber: line,
      endColumn: 100,
    }];
  }

  async function handleRun() {
    if (!result?.success || running) return;
    running = true;
    runResult = null;
    nearReturnDisplay = null;
    nearGasUsed = '';
    nearLogs = [];
    nearPanic = null;
    nearStorageDiff = [];
    nearReceipts = [];
    try {
      if (target === 'p1') {
        // Apply context from UI to the mock runtime
        const rpcUrl = network === 'testnet'
          ? 'https://rpc.testnet.near.org'
          : 'https://rpc.mainnet.near.org';
        setNearContext({ ...nearCtx, rpcUrl });

        // Build input bytes from JSON textarea
        let nearInput: Uint8Array | undefined;
        if (nearInputJson.trim()) {
          nearInput = new TextEncoder().encode(nearInputJson);
        }

        // NEAR contract — run with mocked runtime
        const method = selectedMethod || undefined;
        runResult = method ? `Calling ${method}()...` : 'Running all methods...';
        const nearResult = await runNear(result.wasmBytes!, { method, input: nearInput });
        nearMethods = nearResult.methods;

        // Capture new fields
        nearLogs = nearResult.logs ?? [];
        nearPanic = nearResult.panic ?? null;
        nearStorageDiff = nearResult.storageDiff ?? [];
        nearReceipts = nearResult.receipts ?? [];

        // Format output
        const lines = [nearResult.stdout];

        // Show return value
        const retDecoded = decodeReturnValue(nearResult.returnValue);
        if (retDecoded !== null) {
          nearReturnDisplay = retDecoded;
          lines.push(`Return: ${retDecoded}`);
        }

        // Show gas (static WASM estimation with NEAR pricing)
        nearGasUsed = formatGas(nearResult.gasUsed);
        const bd = nearResult.gasBreakdown;
        if (bd) {
          lines.push(`Gas: ${nearGasUsed} (${bd.opcodes} opcodes, ${formatGas(bd.opcodeGas)} compute, ${formatGas(bd.hostGas)} host)`);
        } else {
          lines.push(`Gas: ${nearGasUsed}`);
        }

        runResult = lines.join('\n');

        // Auto-refresh storage view if open
        if (showNearStorage) {
          nearStorageView = getNearStorage();
        }
      } else if (target === 'p2') {
        // P2 Component → re-compile as core WASM for browser execution
        runResult = 'Compiling core WASM...';
        const coreBytes = compileP2Core(source);
        runResult = 'Running WASI with real HTTP...';
        runResult = await runWasiWithWorker(coreBytes);
      } else {
        runResult = await runPure(result.wasmBytes!);
      }
    } catch (err: unknown) {
      runResult = `Error: ${err instanceof Error ? err.message : String(err)}`;
    } finally {
      running = false;
    }
  }

  // ============================================
  // Test Runner
  // ============================================
  function hasTests(src: string): boolean {
    return /\(test\s+["']/.test(src);
  }

  async function handleRunTests() {
    if (!wasmReady || testing) return;
    
    const { setupCode, tests } = parseTests(source);
    
    if (tests.length === 0) {
      testResults = { tests: [], passed: 0, failed: 0, total: 0 };
      return;
    }
    
    testing = true;
    testResults = null;
    const results: TestRunResult = { tests: [], passed: 0, failed: 0, total: tests.length };
    
    try {
      for (const test of tests) {
        const testCode = buildTestCode(setupCode, test.body);
        
        try {
          // Compile and run
          const res = compile(testCode, 'pure');
          if (res.success && res.wasmBytes) {
            const output = await runPure(res.wasmBytes);
            // WASM traps are caught by runPure and returned as "error: ..." strings.
            // Treat those as test failures.
            if (output.startsWith('error:')) {
              const msg = output === 'error: unreachable' ? 'assertion failed' : output;
              results.tests.push({ name: test.name, passed: false, error: msg });
              results.failed++;
            } else {
              results.tests.push({ name: test.name, passed: true, output });
              results.passed++;
            }
          } else {
            results.tests.push({ name: test.name, passed: false, error: res.error ?? 'Compilation failed' });
            results.failed++;
          }
        } catch (err: unknown) {
          results.tests.push({ 
            name: test.name, 
            passed: false, 
            error: err instanceof Error ? err.message : String(err) 
          });
          results.failed++;
        }
      }
    } finally {
      testing = false;
    }
    
    testResults = results;
  }

  // ============================================
  // Example selection
  // ============================================
  function selectExample(index: number) {
    activeExample = index;
    source = examples[index].source;
    target = examples[index].target;
    if (editorInstance) editorInstance.setValue(source);
    result = null;
    deployResult = null;
    showDeployPanel = false;
    runResult = null;
    testResults = null;
    clearMonacoMarkers();
    saveState();
    updateUrlHash();
    
    // Feature 2: Auto-run on example select (for pure targets)
    if (target === 'pure') {
      setTimeout(() => handleCompile(true), 100);
    }
  }

  // ============================================
  // Wallet
  // ============================================
  async function handleConnectWallet() {
    walletState = await connectWallet(network);
  }

  async function handleDisconnectWallet() {
    await disconnectWallet();
    walletState = getWalletState();
  }

  async function handleDeploy() {
    if (!result?.success || !result.wasmBytes || deploying) return;
    deploying = true;
    deployResult = null;

    try {
      if (target === 'p1') {
        deployResult = await deployP1(result.wasmBytes, contractName, network);
      } else {
        const outlayerId = network === 'testnet' ? 'outlayer.testnet' : 'outlayer.kampouse.near';
        deployResult = await deployP2(result.wasmBytes, outlayerId, network);
      }
    } catch (err: unknown) {
      deployResult = {
        success: false,
        txHash: null,
        explorerUrl: null,
        error: err instanceof Error ? err.message : String(err),
      };
    } finally {
      deploying = false;
    }
  }

  // ============================================
  // REPL Mode (Feature 9)
  // ============================================
  async function handleReplSubmit(e: KeyboardEvent) {
    if (e.key !== 'Enter' || e.shiftKey) return;
    if (!replInput.trim() || !wasmReady) return;
    
    e.preventDefault();
    const expr = replInput.trim();
    replInput = '';
    
    // Wrap singular expression for execution
    const wrappedSource = `(define (main) ${expr})`;
    try {
      const res = compile(wrappedSource, 'pure');
      if (res.success && res.wasmBytes) {
        const val = await runPure(res.wasmBytes);
        replHistory = [...replHistory, { expr, result: val }];
      } else {
        replHistory = [...replHistory, { expr, result: `Error: ${res.error}` }];
      }
    } catch (err: unknown) {
      replHistory = [...replHistory, { expr, result: `Error: ${err instanceof Error ? err.message : String(err)}` }];
    }
    
    // Scroll to bottom
    setTimeout(() => {
      const output = document.querySelector('.repl-output');
      if (output) output.scrollTop = output.scrollHeight;
    }, 10);
  }

  // ============================================
  // Feature 5: Keyboard shortcut (Cmd/Ctrl+Enter)
  // ============================================
  function handleGlobalKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      handleCompile(false);
    }
  }

  // ============================================
  // Lifecycle
  // ============================================
  onMount(() => {
    setupMonaco();

    const loadWasm = async () => {
      try {
        await initCompiler();
        wasmReady = true;
      } catch (err) {
        console.error('Failed to initialize WASM:', err);
      }
    };
    loadWasm();

    // Check for existing wallet connection
    walletState = getWalletState();

    // Feature 7: Load from URL hash
    const urlState = loadFromUrl();
    
    // Load virtual file system
    files = loadFiles();
    
    // Feature 8: Load from localStorage
    const storedState = loadState();
    
    // Priority: URL > files > localStorage > default
    if (urlState.source) {
      source = urlState.source;
      if (urlState.target) target = urlState.target;
      if (editorInstance) editorInstance.setValue(source);
      // Save URL code into current file or create one
      if (files.length === 0) createFile('shared.lisp');
      if (activeFileId) saveCurrentFile();
    } else if (files.length > 0) {
      // Restore active file from stored state
      const savedFileId = storedState?.activeFileId;
      const targetFile = (savedFileId && files.find(f => f.id === savedFileId)) || files[0];
      activeFileId = targetFile.id;
      source = targetFile.source;
      target = targetFile.target;
      if (editorInstance) editorInstance.setValue(source);
    } else if (storedState) {
      source = storedState.source;
      target = storedState.target;
      activeExample = storedState.activeExample;
      autoCompile = storedState.autoCompile;
      replMode = storedState.replMode;
      if (editorInstance) editorInstance.setValue(source);
      // Migrate existing code into a file
      const file: VFile = { id: generateId(), name: 'main.lisp', source, target, updatedAt: Date.now() };
      files = [file];
      activeFileId = file.id;
      saveFiles();
    } else {
      source = examples[0].source;
      target = examples[0].target;
      // Create default file
      const file: VFile = { id: generateId(), name: 'main.lisp', source, target, updatedAt: Date.now() };
      files = [file];
      activeFileId = file.id;
      saveFiles();
    }

    // Feature 5: Global keyboard shortcut
    document.addEventListener('keydown', handleGlobalKeydown);

    return () => {
      editorInstance?.dispose();
      document.removeEventListener('keydown', handleGlobalKeydown);
    };
  });

  onDestroy(() => {
    if (compileDebounceTimer) clearTimeout(compileDebounceTimer);
  });

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  }

  function formatTime(ms: number): string {
    if (ms < 1) return `${ms.toFixed(2)} ms`;
    if (ms < 1000) return `${ms.toFixed(1)} ms`;
    return `${(ms / 1000).toFixed(2)} s`;
  }

  // Feature 7: Copy share URL
  async function copyShareUrl() {
    updateUrlHash();
    const url = window.location.href;
    try {
      await navigator.clipboard.writeText(url);
      // Could add a toast notification here
    } catch {
      // Fallback: show URL in prompt
      prompt('Share this URL:', url);
    }
  }
</script>

<div class="app-container">
  <!-- Loading Overlay -->
  {#if !wasmReady}
    <div class="loading-overlay">
      <div class="loading-spinner"></div>
      <div class="loading-text">Initializing Compiler</div>
      <div class="loading-sub">Loading WebAssembly module...</div>
    </div>
  {/if}

  <!-- Mobile Examples Drawer -->
  {#if showExamplesMenu}
    <div class="drawer-overlay" onclick={() => { showExamplesMenu = false; }}></div>
    <div class="drawer">
      <div class="drawer-header">
        <span>Examples</span>
        <button class="drawer-close" onclick={() => { showExamplesMenu = false; }}>
          <X size={18} />
        </button>
      </div>
      <div class="drawer-content">
        {#each examples as example, i}
          <button
            class="drawer-item"
            class:active={activeExample === i}
            onclick={() => { selectExample(i); showExamplesMenu = false; }}
          >
            <span class="example-icon">{example.icon}</span>
            {example.name}
          </button>
        {/each}
        <div class="drawer-divider"></div>
        <button
          class="drawer-item"
          class:active={replMode}
          onclick={() => { replMode = !replMode; saveState(); showExamplesMenu = false; }}
        >
          <Zap size={16} />
          REPL Mode
        </button>
        {#if result?.success}
          <div class="drawer-divider"></div>
          <button
            class="drawer-item"
            onclick={() => { showDeployPanel = !showDeployPanel; deployResult = null; showExamplesMenu = false; }}
          >
            <Rocket size={16} />
            Deploy to {target === 'p1' ? 'NEAR' : 'OutLayer'}
          </button>
        {/if}
        <div class="drawer-divider"></div>
        {#if walletState.connected}
          <button
            class="drawer-item"
            onclick={() => { handleDisconnectWallet(); showExamplesMenu = false; }}
          >
            <CircleDot size={16} />
            Disconnect ({shortAccountId || walletState.accountId?.slice(0, 8)}...)
          </button>
        {:else}
          <button
            class="drawer-item"
            onclick={() => { handleConnectWallet(); showExamplesMenu = false; }}
          >
            <Wallet size={16} />
            Connect Wallet
          </button>
        {/if}
        <div class="drawer-divider"></div>
        <button
          class="drawer-item"
          class:active={showLearn}
          onclick={() => { showLearn = !showLearn; showExamplesMenu = false; }}
        >
          <BookOpen size={16} />
          Learn
        </button>
      </div>
    </div>
  {/if}

  <!-- Fixed Header -->
  <header class="header">
    <!-- Mobile menu button -->
    <button class="mobile-menu-btn" onclick={() => { showExamplesMenu = !showExamplesMenu; }}>
      {#if showExamplesMenu}
        <X size={20} />
      {:else}
        <Menu size={20} />
      {/if}
    </button>

    <div class="header-brand" onclick={() => { showLearn = false; }} role="button" tabindex="0">
      <div class="header-logo">λ</div>
      <span class="header-title">Lisp → WASM</span>
    </div>

    <div class="pill-container" role="tablist">
      <button
        class="pill-tab"
        class:active={target === 'pure'}
        role="tab"
        aria-selected={target === 'pure'}
        onclick={() => { target = 'pure'; saveState(); if (autoCompile) scheduleCompile(); }}
      >
        <Zap size={14} /> <span class="pill-label">Run</span>
      </button>
      <button
        class="pill-tab"
        class:active={target === 'p1'}
        role="tab"
        aria-selected={target === 'p1'}
        onclick={() => { target = 'p1'; saveState(); if (autoCompile) scheduleCompile(); }}
      >
        <Box size={14} /> <span class="pill-label">NEAR</span>
      </button>
      <button
        class="pill-tab"
        class:active={target === 'p2'}
        role="tab"
        aria-selected={target === 'p2'}
        onclick={() => { target = 'p2'; saveState(); if (autoCompile) scheduleCompile(); }}
      >
        <Cloud size={14} /> <span class="pill-label">WASI</span>
      </button>
    </div>

    <!-- Feature 5: Auto-compile toggle & Feature 7: Share -->
    <button
      class="header-toggle"
      class:active={autoCompile}
      onclick={() => { autoCompile = !autoCompile; saveState(); }}
      title="Auto-compile on type (debounced 300ms)"
    >
      <Zap size={14} />
      Auto
    </button>

    <button
      class="header-icon-btn"
      onclick={copyShareUrl}
      title="Copy shareable URL"
    >
      <Link size={16} />
    </button>

    <!-- Network toggle -->
    <button
      class="network-badge"
      onclick={() => { network = network === 'testnet' ? 'mainnet' : 'testnet'; }}
      title="Switch network"
    >
      <FlaskConical size={12} />
      {network}
    </button>

    <!-- Wallet button -->
    {#if walletState.connected}
      <button class="wallet-btn connected" onclick={handleDisconnectWallet} title={walletState.accountId ?? ''}>
        <CircleDot size={12} />
        {shortAccountId}
      </button>
    {:else}
      <button class="wallet-btn" onclick={handleConnectWallet}>
        <Wallet size={14} />
        Connect
      </button>
    {/if}

    <button
      class="header-compile-btn"
      class:compiling={compiling}
      disabled={!wasmReady || compiling}
      onclick={() => handleCompile(false)}
      title="Compile"
    >
      {#if compiling}
        <Loader2 size={16} class="spinner-icon" />
      {:else}
        <Hammer size={16} />
      {/if}
    </button>

    <button
      class="header-run-btn"
      onclick={handleRun}
      disabled={!result?.success || running}
      title="Run"
    >
      {#if running}
        <Loader2 size={16} class="spinner-icon" />
      {:else}
        <Play size={16} />
      {/if}
    </button>

    <button
      class="header-test-btn"
      onclick={handleRunTests}
      disabled={!wasmReady || testing}
      title="Run tests"
    >
      {#if testing}
        <Loader2 size={16} class="spinner-icon" />
      {:else}
        <CheckCircle size={16} />
      {/if}
    </button>
  </header>

  <!-- Learn Panel (overlay — workbench stays mounted underneath) -->
  {#if showLearn}
    <div class="learn-overlay">
      <div class="learn-overlay-header">
        <span class="learn-overlay-title">Learn</span>
        <button class="learn-overlay-close" onclick={() => { showLearn = false; }}>
          <X size={18} />
        </button>
      </div>
      <div class="learn-panel">
      <div class="learn-content">
        <div class="learn-section">
          <h3>Lisp Basics</h3>
          <p>Lisp uses <strong>prefix notation</strong> — the operator comes first:</p>
          <pre class="learn-code">(+ 1 2)       ; → 3
(* 3 4)       ; → 12
(- 10 3)      ; → 7
(/ 20 4)      ; → 5
(mod 17 5)    ; → 2 (modulo)</pre>
          <p>Nest expressions for complex calculations:</p>
          <pre class="learn-code">(+ (* 2 3) (- 10 5))  ; → 11</pre>
        </div>

        <div class="learn-section">
          <h3>Variables & Functions</h3>
          <p>Define variables with <code>let</code>:</p>
          <pre class="learn-code">(let ((x 10) (y 20))
  (+ x y))     ; → 30</pre>
          <p>Define functions with <code>defun</code>:</p>
          <pre class="learn-code">(defun square (n)
  (* n n))

(square 5)    ; → 25

(defun factorial (n)
  (if (&lt;= n 1)
      1
      (* n (factorial (- n 1)))))

(factorial 5)  ; → 120</pre>
        </div>

        <div class="learn-section">
          <h3>Conditionals</h3>
          <pre class="learn-code">(if (> x 0)
    "positive"
    "non-positive")</pre>
          <p>Multi-branch with <code>cond</code>:</p>
          <pre class="learn-code">(cond
  ((&lt; n 0) "negative")
  ((= n 0) "zero")
  (else "positive"))</pre>
          <p>Compare with: <code>=</code> <code>!=</code> <code>&lt;</code> <code>&gt;</code> <code>&lt;=</code> <code>&gt;=</code></p>
        </div>

        <div class="learn-section">
          <h3>Lists & Pairs</h3>
          <pre class="learn-code">(list 1 2 3)        ; → (1 2 3)
(car (list 1 2 3))  ; → 1 (first element)
(cdr (list 1 2 3))  ; → (2 3) (rest)
(cons 0 (list 1 2)) ; → (0 1 2)
(length (list 1 2 3)) ; → 3
(null? '())         ; → true (empty list check)</pre>
        </div>

        <div class="learn-section">
          <h3>String Functions</h3>
          <pre class="learn-code">(str-concat "hello" " " "world")  ; → "hello world"
(str-length "hello")             ; → 5
(string-ref "hello" 1)            ; → "e"
(str-upcase "hello")          ; → "HELLO"
(str-downcase "HELLO")        ; → "hello"
(substring "hello" 1 4)       ; → "ell"</pre>
        </div>

        <div class="learn-section">
          <h3>Higher-Order Functions</h3>
          <pre class="learn-code">(map (lambda (x) (* x x)) (list 1 2 3 4))
; → (1 4 9 16)

(filter (lambda (x) (> x 2)) (list 1 2 3 4))
; → (3 4)

(fold-left + 0 (list 1 2 3 4))
; → 10</pre>
        </div>

        <div class="learn-section">
          <h3>Execution Modes</h3>
          <p>Lisp compiles to WASM and runs in three distinct environments — each with a specific purpose:</p>
        </div>

        <div class="learn-section">
          <h3>⚡ Run — Pure Browser Execution</h3>
          <div class="learn-modes">
            <div class="learn-mode">
              <p><strong>Why it exists:</strong> Instant feedback without blockchain overhead. Zero gas, zero waiting, zero wallet.</p>
              <p><strong>Best for:</strong> Learning Lisp, prototyping algorithms, testing logic before deploying on-chain.</p>
              <p><strong>How it works:</strong> Compiles to WASM and executes in a Web Worker. Pure computation — function calls only, deterministic results.</p>
              <p><strong>Limitations:</strong> No side effects. You <strong>cannot</strong> use:</p>
              <ul>
                <li><code>storage-set</code> / <code>storage-get</code> — no persistent state</li>
                <li><code>http-get</code> / <code>http-post</code> — no network calls</li>
                <li><code>block-height</code> / <code>signer-account-id</code> — no blockchain context</li>
              </ul>
              <p>Pure mode is for <em>computing results</em> only. State and I/O require NEAR or WASI mode.</p>
            </div>
          </div>
        </div>

        <div class="learn-section">
          <h3>📦 NEAR — On-Chain Smart Contracts</h3>
          <div class="learn-modes">
            <div class="learn-mode">
              <p><strong>Why it exists:</strong> Deploy code that owns state and money. Trustless, permanent, composable.</p>
              <p><strong>Best for:</strong> DeFi protocols, NFT contracts, DAOs, payment logic — anything needing economic security.</p>
              <p><strong>How it works:</strong> Compiles to WASM and deploys to NEAR. Your code becomes an on-chain account with persistent storage. Gas fees apply.</p>
            </div>
          </div>
          <pre class="learn-code">;; Example: Counter contract with state
(defvar *counter* 0)

(defun increment ()
  (set! *counter* (+ *counter* 1))
  *counter*)

(defun get-counter ()
  *counter*)</pre>
        </div>

        <div class="learn-section">
          <h3>☁️ WASI — Off-Chain Compute</h3>
          <div class="learn-modes">
            <div class="learn-mode">
              <p><strong>Why it exists:</strong> Heavy computation is expensive on-chain. WASI runs off-chain with HTTP and storage — no gas limits.</p>
              <p><strong>Best for:</strong> API oracles, data processing, AI inference, complex math, large-scale computations.</p>
              <p><strong>How it works:</strong> Compiles to WASM with WASI extensions, runs via OutLayer. Can call HTTP APIs and use persistent storage. Results are verifiable on-chain.</p>
            </div>
          </div>
          <pre class="learn-code">;; Example: Fetch external API
(defun fetch-price ()
  (let ((response (http-get "https://api.coinbase.com/v2/prices/BTC-USD/spot")))
    (json-parse response)))

;; Example: P2 Storage
(defun main ()
  (begin
    (storage-set "key" "value")
    (storage-get "key")))  ; → "value"</pre>
        </div>

        <div class="learn-section">
          <h3>✓ Testing</h3>
          <p>Write tests to verify your code works as expected. Use the Test button (✓) in the header to run tests.</p>
          <pre class="learn-code">;; Define a function
(defun add (a b)
  (+ a b))

;; Write tests with (test "name" body...)
(test "addition works"
  (assert-equal 5 (add 2 3)))

(test "handles zero"
  (assert-equal 0 (add 0 0))
  (assert-equal 5 (add 5 0)))

(test "negative numbers"
  (assert-equal -2 (add -5 3))
  (assert-equal -8 (add -5 -3)))</pre>
          <p><strong>Assertion functions:</strong></p>
          <ul>
            <li><code>(assert-equal expected actual)</code> — fails if values don't match</li>
            <li><code>(assert-true expr)</code> — fails if expression is false</li>
            <li><code>(assert-false expr)</code> — fails if expression is true</li>
          </ul>
          <p>Tests run in the selected mode (Run/NEAR/WASI). Use Run mode for fastest feedback during development.</p>
        </div>

        <div class="learn-section">
          <h3>Available Functions</h3>
          <div class="learn-functions">
            <div class="learn-fn-group">
              <strong>Arithmetic</strong>
              <code>+ - * / mod abs min max</code>
            </div>
            <div class="learn-fn-group">
              <strong>Comparison</strong>
              <code>= != &lt; &gt; &lt;= &gt;=</code>
            </div>
            <div class="learn-fn-group">
              <strong>Logic</strong>
              <code>and or not</code>
            </div>
            <div class="learn-fn-group">
              <strong>Control</strong>
              <code>if cond let defun lambda set!</code>
            </div>
            <div class="learn-fn-group">
              <strong>Lists</strong>
              <code>car cdr cons list length null? append reverse</code>
            </div>
            <div class="learn-fn-group">
              <strong>Strings</strong>
              <code>str-concat str-length substring str-upcase str-downcase string-ref</code>
            </div>
            <div class="learn-fn-group">
              <strong>Higher-Order</strong>
              <code>map filter fold-left fold-right apply</code>
            </div>
            <div class="learn-fn-group">
              <strong>Predicates</strong>
              <code>null? list? number? string? symbol?</code>
            </div>
            <div class="learn-fn-group">
              <strong>WASI Only</strong>
              <code>http-get http-post json-parse from-json</code>
            </div>
            <div class="learn-fn-group">
              <strong>P2 Storage</strong>
              <code>storage-set storage-get storage-has storage-delete</code>
            </div>
          </div>
        </div>

        <div class="learn-section">
          <h3>Examples</h3>
          <p>Click the hamburger menu (☰) to explore built-in examples: Fibonacci, factorial, list operations, and more.</p>
        </div>
      </div>
    </div>
    </div>
  {/if}

  <!-- Main Content - Split Layout — always mounted -->
    <main class="main-content" class:workbench-hidden={showLearn}>
      <div class="split-container">
        <!-- Sidebar (Files + Outline) -->
        {#if showOutline}
          <aside class="outline-panel">
            <button class="outline-section-header" onclick={() => showApiRef = !showApiRef}>
              <span class="outline-title">API · {target.toUpperCase()}</span>
              <span class="api-toggle">{showApiRef ? '▾' : '▸'}</span>
            </button>
            {#if showApiRef}
              <div class="api-ref-body">
                {#each apiForTarget as group}
                  <div class="api-group">
                    <div class="api-group-title" onclick={() => apiExpanded[group.title] = !apiExpanded[group.title]}>
                      <span>{apiExpanded[group.title] ? '▾' : '▸'} {group.title}</span>
                      <span class="api-count">{group.items.length}</span>
                    </div>
                    {#if apiExpanded[group.title]}
                      <div class="api-items">
                        {#each group.items as fn}
                          <span class="api-fn" onclick={() => insertSnippet(fn)}>{fn}</span>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            <div class="outline-divider"></div>

            <div class="outline-header">
              <span class="outline-title">FILES</span>
              <div class="outline-header-actions">
                <button class="outline-action-btn" onclick={() => createFile()} title="New file">
                  +
                </button>
                <button class="outline-toggle" onclick={() => { showOutline = false; }} title="Hide sidebar">
                  <ChevronRight size={14} />
                </button>
              </div>
            </div>
            <div class="outline-files">
              {#each files as file}
                {#if renamingFileId === file.id}
                  <div class="file-item renaming">
                    <input
                      class="file-rename-input"
                      type="text"
                      bind:value={renameInput}
                      onkeydown={(e) => { if (e.key === 'Enter') commitRename(); if (e.key === 'Escape') { renamingFileId = null; } }}
                      onblur={commitRename}
                    />
                  </div>
                {:else}
                  <button
                    class="file-item"
                    class:active={file.id === activeFileId}
                    onclick={() => switchToFile(file.id)}
                    oncontextmenu={(e) => handleFileContextMenu(e, file.id)}
                    title={file.name}
                  >
                    <span class="file-icon">λ</span>
                    <span class="file-name">{file.name}</span>
                  </button>
                {/if}
              {/each}
            </div>

            <div class="outline-divider"></div>

            <div class="outline-section-header">
              <span class="outline-title">OUTLINE</span>
            </div>
            <div class="outline-body">
              {#each outlineItems as item}
                <button
                  class="outline-item outline-{item.kind}"
                  onclick={() => jumpToLine(item.line)}
                  title="Line {item.line}"
                >
                  <span class="outline-icon">{outlineIcon(item.kind)}</span>
                  <span class="outline-name">{item.name}</span>
                  <span class="outline-line">{item.line}</span>
                </button>
              {:else}
                <div class="outline-empty">No symbols found</div>
              {/each}
            </div>
          </aside>
        {:else}
          <button class="outline-collapsed-btn" onclick={() => { showOutline = true; }} title="Show sidebar">
            <FileCode size={14} />
          </button>
        {/if}

        <!-- File Context Menu -->
        {#if fileContextMenu}
          <div class="ctx-overlay" onclick={closeContextMenu}></div>
          <div class="ctx-menu" style="left: {fileContextMenu.x}px; top: {fileContextMenu.y}px;">
            <button class="ctx-item" onclick={() => startRename(fileContextMenu!.fileId)}>
              Rename
            </button>
            <button class="ctx-item" onclick={() => duplicateFile(fileContextMenu!.fileId)}>
              Duplicate
            </button>
            <div class="ctx-divider"></div>
            <button class="ctx-item ctx-danger" onclick={() => deleteFile(fileContextMenu!.fileId)}>
              Delete
            </button>
          </div>
        {/if}

        <!-- Editor Pane -->
        <div class="editor-pane" class:full-height={outputCollapsed} style="flex: 1 1 {outputCollapsed ? '100' : '60'}%;">
          <section class="editor-section">
            <div class="editor-wrapper">
              <div class="editor-container" bind:this={editorContainer}></div>
            </div>

          <!-- Examples + REPL toggle -->
          {#if !outputCollapsed}
            <div class="examples-bar">
              {#each examples as example, i}
                <button
                  class="example-btn"
                  class:active={activeExample === i}
                  onclick={() => selectExample(i)}
                >
                  <span class="example-icon">{example.icon}</span>
                  {example.name}
                </button>
              {/each}

              <!-- Feature 9: REPL mode toggle -->
              <button
                class="example-btn repl-toggle"
                class:active={replMode}
                onclick={() => { replMode = !replMode; saveState(); }}
                title="Toggle REPL mode"
              >
                <Zap size={14} />
                REPL
              </button>
            </div>
          {/if}
        </section>
      </div>

      <!-- Output Pane Toggle (when collapsed) - desktop only -->
      {#if outputCollapsed}
        <button
          class="output-fab"
          onclick={() => { outputCollapsed = false; }}
          title="Show output"
        >
          <ChevronUp size={18} />
        </button>
      {:else}
        <!-- Resizer -->
        <div class="resizer" onmousedown={startResize}></div>

        <!-- Output Pane -->
        <div class="output-pane" style="width: {outputPaneWidth}%;">
        {#if replMode}
          <section class="output-section repl-mode">
            <div class="repl-panel">
              <div class="repl-output">
                {#each replHistory as entry}
                  <div class="repl-line">
                    <span class="repl-prompt">></span>
                    <span class="repl-expr">{entry.expr}</span>
                  </div>
                  <div class="repl-result-line">
                    <span class="repl-result-val">{entry.result}</span>
                  </div>
                {:else}
                  <div class="repl-line" style="color: var(--color-text-muted)">
                    // Enter a Lisp expression and press Enter
                  </div>
                {/each}
              </div>
              <div class="repl-input-row">
                <span class="repl-prompt">></span>
                <input
                  type="text"
                  class="repl-input"
                  bind:value={replInput}
                  onkeydown={handleReplSubmit}
                  placeholder="(+ 1 2)"
                  disabled={!wasmReady}
                />
              </div>
            </div>
          </section>
        {:else}
          <section class="output-section">
            <div class="output-panel" class:animate-slide-up={result}>
              <div class="output-header">
                <button
                  class="collapse-toggle"
                  onclick={() => { outputCollapsed = !outputCollapsed; }}
                  title={outputCollapsed ? 'Expand output' : 'Collapse output'}
                >
                  {#if outputCollapsed}
                    <ChevronDown size={16} />
                  {:else}
                    <ChevronUp size={16} />
                  {/if}
                </button>
                <span class="output-title">Output</span>
              </div>

              <div class="output-body">
                {#if result?.success}
                  <!-- Run Result -->
                  {#if runResult !== null}
                    <div class="run-result-panel" class:info={runResult.startsWith('ℹ')}>
                      <div class="run-result-header">
                        <span class="run-result-icon">{runResult.startsWith('Error') ? '✗' : '▶'}</span>
                        <span class="run-result-title">Output</span>
                      </div>
                      <div class="run-result-value" class:error={runResult.startsWith('Error')} class:info-text={runResult.startsWith('ℹ')}>
                        <pre>{runResult}</pre>
                      </div>
                    </div>
                  {/if}

                  <!-- Test Results -->
                  {#if testResults !== null}
                    <div class="test-results-panel">
                      <div class="test-results-header">
                        <span class="test-results-icon">{testResults.failed === 0 ? '✓' : '✗'}</span>
                        <span class="test-results-title">
                          {testResults.passed}/{testResults.total} tests passed
                        </span>
                        {#if testResults.failed > 0}
                          <span class="test-count-failed">{testResults.failed} failed</span>
                        {/if}
                      </div>
                      <div class="test-results-list">
                        {#each testResults.tests as test}
                          <div class="test-item" class:passed={test.passed} class:failed={!test.passed}>
                            <span class="test-status-icon">{test.passed ? '✓' : '✗'}</span>
                            <span class="test-name">{test.name}</span>
                            {#if !test.passed && test.error}
                              <span class="test-error">{test.error}</span>
                            {/if}
                          </div>
                        {/each}
                      </div>
                    </div>
                  {/if}

                  <!-- NEAR Method Runner -->
                  {#if target === 'p1' && result?.success}
                    <div class="near-method-panel">
                      <div class="near-method-header">
                        <span class="near-method-title">Call Method</span>
                        <button class="near-context-toggle" onclick={() => { showNearContext = !showNearContext; }} title="Configure context">
                          <Hammer size={14} />
                        </button>
                      </div>

                      <!-- Context config -->
                      {#if showNearContext}
                        <div class="near-context-form">
                          <div class="near-ctx-row">
                            <label>Signer</label>
                            <input type="text" bind:value={nearCtx.signerAccount} placeholder="user.testnet" onchange={() => setNearContext(nearCtx)} />
                          </div>
                          <div class="near-ctx-row">
                            <label>Deposit</label>
                            <input type="text" value={nearCtx.attachedDeposit.toString()} oninput={(e) => { try { nearCtx.attachedDeposit = BigInt((e.target as HTMLInputElement).value); setNearContext(nearCtx); } catch {} }} placeholder="0" />
                            <span class="near-ctx-unit">yoctoⓃ</span>
                          </div>
                          <div class="near-ctx-row">
                            <label>Balance</label>
                            <input type="text" value={nearCtx.accountBalance.toString()} oninput={(e) => { try { nearCtx.accountBalance = BigInt((e.target as HTMLInputElement).value); setNearContext(nearCtx); } catch {} }} placeholder="1000000..." />
                            <span class="near-ctx-unit">yoctoⓃ</span>
                          </div>
                          <div class="near-ctx-row">
                            <label>Block</label>
                            <input type="text" value={nearCtx.blockIndex.toString()} oninput={(e) => { try { nearCtx.blockIndex = BigInt((e.target as HTMLInputElement).value); setNearContext(nearCtx); } catch {} }} placeholder="12345678" />
                          </div>
                        </div>
                      {/if}

                      <!-- Method selector -->
                      <div class="near-method-actions">
                        <select class="near-method-select" bind:value={selectedMethod}>
                          <option value="">— all methods —</option>
                          {#each nearMethods as m}
                            <option value={m}>{m}()</option>
                          {/each}
                        </select>
                        <button class="near-method-run" onclick={handleRun} disabled={running}>
                          {running ? '...' : 'Run'}
                        </button>
                      </div>

                      <!-- Input arguments (JSON) -->
                      <div class="near-input-area">
                        <textarea
                          class="near-input-textarea"
                          bind:value={nearInputJson}
                          placeholder={'{"account_id": "bob.near"}'}
                          rows="2"
                          spellcheck="false"
                        ></textarea>
                      </div>

                      <!-- Return value + gas -->
                      {#if nearReturnDisplay !== null || nearGasUsed}
                        <div class="near-result-bar">
                          {#if nearReturnDisplay !== null}
                            <span class="near-result-ret">→ {nearReturnDisplay}</span>
                          {/if}
                          {#if nearGasUsed}
                            <span class="near-result-gas">⛽ {nearGasUsed}</span>
                          {/if}
                        </div>
                      {/if}

                      <!-- Panic -->
                      {#if nearPanic}
                        <div class="near-panic">
                          <span class="near-panic-icon">⚠</span>
                          <span class="near-panic-msg">{nearPanic}</span>
                        </div>
                      {/if}

                      <!-- Logs -->
                      {#if nearLogs.length > 0}
                        <div class="near-logs-section">
                          <div class="near-logs-title">Logs</div>
                          {#each nearLogs as log, i}
                            <div class="near-log-entry">
                              <span class="near-log-prefix">[{i}]</span> {log}
                            </div>
                          {/each}
                        </div>
                      {/if}

                      <!-- Storage Diff -->
                      {#if nearStorageDiff.length > 0}
                        <div class="near-diff-section">
                          <div class="near-diff-title">Storage Changes</div>
                          {#each nearStorageDiff as diff}
                            <div class="near-diff-entry">
                              <span class="near-diff-key">{diff.key}</span>
                              <span class="near-diff-arrow">→</span>
                              <span class="near-diff-old">{diff.oldVal ?? '∅'}</span>
                              <span class="near-diff-slash">/</span>
                              <span class="near-diff-new">{diff.newVal ?? '∅'}</span>
                            </div>
                          {/each}
                        </div>
                      {/if}

                      <!-- Receipts (Promise DAG) -->
                      {#if nearReceipts.length > 0}
                        <div class="near-receipts-section">
                          <div class="near-receipts-title">Cross-Contract Calls</div>
                          {#each nearReceipts as receipt}
                            <div class="near-receipt-entry">
                              <span class="near-receipt-method">{receipt.accountId}.{receipt.methodName}()</span>
                              <span class="near-receipt-meta">args: {receipt.argsSize}B · {receipt.type}</span>
                              {#if receipt.result}
                                <span class="near-receipt-result">{new TextDecoder().decode(receipt.result)}</span>
                              {/if}
                            </div>
                          {/each}
                        </div>
                      {/if}
                    </div>
                  {/if}

                  <!-- NEAR Storage Inspector -->
                  {#if target === 'p1' && result?.success}
                    <div class="near-storage-panel">
                      <div class="near-storage-header">
                        <span class="near-storage-title">Contract State</span>
                        <div class="near-storage-actions">
                          <button class="near-storage-btn" onclick={() => { nearStorageView = getNearStorage(); showNearStorage = true; }} title="View storage">
                            <Database size={14} />
                          </button>
                          <button class="near-storage-btn near-storage-btn-danger" onclick={() => { clearNearStorage(); nearStorageView = {}; showNearStorage = false; }} title="Clear storage">
                            <Trash2 size={14} />
                          </button>
                        </div>
                      </div>
                      {#if showNearStorage && nearStorageView !== null}
                        <div class="near-storage-content">
                          {#if Object.keys(nearStorageView).length === 0}
                            <div class="near-storage-empty">Empty — no keys stored yet</div>
                          {:else}
                            {#each Object.entries(nearStorageView) as [key, value]}
                              <div class="near-storage-entry">
                                <span class="near-storage-key">{key}</span>
                                <span class="near-storage-value">{value}</span>
                              </div>
                            {/each}
                          {/if}
                        </div>
                      {/if}
                    </div>
                  {/if}

                  <!-- WAT Disassembly -->
                  {#if result.wat}
                    <details class="hex-details">
                      <summary class="hex-summary">
                        WAT Disassembly
                      </summary>
                      <pre class="wat-output">{result.wat}</pre>
                    </details>
                  {/if}

                  <!-- Deploy Panel -->
                  {#if showDeployPanel}
                    <div class="deploy-panel">
                      <div class="deploy-header">
                        <span>Deploy to {target === 'p1' ? 'NEAR' : 'OutLayer'}</span>
                      </div>
                      {#if !walletState.connected}
                        <div class="deploy-wallet-prompt">
                          <p>Connect your NEAR wallet to deploy</p>
                          <button class="deploy-connect-btn" onclick={handleConnectWallet}>
                            Connect Wallet
                          </button>
                        </div>
                      {:else}
                        <div class="deploy-form">
                          {#if target === 'p1'}
                            <div class="deploy-field">
                              <label class="deploy-label">Contract Name</label>
                              <div class="deploy-input-group">
                                <input
                                  type="text"
                                  class="deploy-input"
                                  bind:value={contractName}
                                  placeholder="my-contract"
                                />
                                <span class="deploy-suffix">.{walletState.accountId}</span>
                              </div>
                            </div>
                          {:else}
                            <div class="deploy-field">
                              <label class="deploy-label">OutLayer Contract</label>
                              <span class="deploy-readonly">
                                {network === 'testnet' ? 'outlayer.testnet' : 'outlayer.kampouse.near'}
                              </span>
                            </div>
                          {/if}
                          <button
                            class="deploy-btn"
                            onclick={handleDeploy}
                            disabled={deploying}
                          >
                            {#if deploying}
                              <span class="spinner"></span>
                              Deploying...
                            {:else}
                              🚀 Deploy to {target === 'p1' ? 'NEAR' : 'OutLayer'}
                            {/if}
                          </button>
                        </div>
                      {/if}

                      <!-- Deploy Result -->
                      {#if deployResult}
                        <div class="deploy-result" class:success={deployResult.success} class:error={!deployResult.success}>
                          {#if deployResult.success}
                            <div class="deploy-result-icon">✅</div>
                            <div class="deploy-result-text">{target === 'p2' ? 'Execution submitted!' : 'Contract deployed!'}</div>
                            {#if target === 'p2' && deployResult.fastfsUrl}
                              <div class="deploy-detail">
                                <span class="deploy-detail-label">FastFS</span>
                                <a href={deployResult.fastfsUrl} target="_blank" rel="noopener" class="deploy-tx-link">
                                  {deployResult.wasmHash?.slice(0, 12)}… →
                                </a>
                              </div>
                            {/if}
                            {#if target === 'p2' && deployResult.wasmHash}
                              <div class="deploy-detail">
                                <span class="deploy-detail-label">SHA-256</span>
                                <code class="deploy-hash">{deployResult.wasmHash.slice(0, 16)}…</code>
                              </div>
                            {/if}
                            {#if deployResult.explorerUrl}
                              <a
                                class="deploy-tx-link"
                                href={deployResult.explorerUrl}
                                target="_blank"
                                rel="noopener"
                              >
                                View on Explorer →
                              </a>
                            {/if}
                          {:else}
                            <div class="deploy-result-icon">❌</div>
                            <div class="deploy-result-text">{deployResult.error}</div>
                          {/if}
                        </div>
                      {/if}
                    </div>
                  {/if}

                  <!-- Stats Grid - at bottom -->
                  <div class="stats-grid">
                    <div class="stat-item">
                      <span class="stat-label">WASM SIZE</span>
                      <span class="stat-value">{formatSize(result.size)}</span>
                    </div>
                    <div class="stat-item">
                      <span class="stat-label">COMPILE TIME</span>
                      <span class="stat-value">{formatTime(result.timeMs)}</span>
                    </div>
                    <div class="stat-item">
                      <span class="stat-label">TARGET</span>
                      <span class="stat-value">{target.toUpperCase()}</span>
                    </div>
                    <div class="stat-item">
                      <span class="stat-label">BYTES</span>
                      <span class="stat-value">{result.size.toLocaleString()}</span>
                    </div>
                  </div>
                {:else if result}
                  <div class="error-message">{result.error}</div>
                {/if}
              </div>
            </div>
          </section>
        {/if}
      </div>
      {/if}
    </div>

    <!-- Mobile bottom toggle bar - always visible on mobile -->
    <div class="mobile-toggle-bar">
      <button
        class="mobile-toggle-btn"
        onclick={() => { outputCollapsed = !outputCollapsed; }}
      >
        <span class="toggle-label">Output</span>
        {#if outputCollapsed}
          <ChevronUp size={18} />
        {:else}
          <ChevronDown size={18} />
        {/if}
      </button>
    </div>
  </main>

  <footer class="footer">
    Lisp RLM — Write Lisp, Deploy Smart Contracts
  </footer>
</div>

<style>
  /* Error Lens — inline error/warning messages (using :global for Monaco DOM) */
  :global(.error-lens-line-error) {
    background: rgba(255, 40, 40, 0.12) !important;
  }
  :global(.error-lens-line-warning) {
    background: rgba(255, 170, 0, 0.10) !important;
  }
  :global(.error-lens-inline-error) {
    color: #ff6b6b;
    font-style: italic;
    font-size: 12px;
    opacity: 0.9;
  }
  :global(.error-lens-inline-warning) {
    color: #ffaa44;
    font-style: italic;
    font-size: 12px;
    opacity: 0.9;
  }

  /* Code Outline */
  .outline-panel {
    width: 180px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--color-border);
    background: var(--color-bg-elevated);
    overflow: hidden;
  }
  .outline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .outline-title {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1px;
    color: var(--color-text-muted);
  }
  .outline-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2px;
    background: transparent;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .outline-toggle:hover {
    color: var(--color-text);
    background: var(--color-bg-surface);
  }
  .outline-body {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }
  .outline-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    background: transparent;
    border: none;
    color: var(--color-text-secondary);
    font-size: 12px;
    font-family: 'JetBrains Mono', monospace;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }
  .outline-item:hover {
    background: var(--color-bg-surface);
    color: var(--color-text-primary);
  }
  .outline-icon {
    width: 16px;
    text-align: center;
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 600;
  }
  .outline-function .outline-icon,
  .outline-define .outline-icon {
    color: #c792ea;
  }
  .outline-variable .outline-icon {
    color: #f29e74;
  }
  .outline-test .outline-icon {
    color: var(--color-accent);
  }
  .outline-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .outline-line {
    color: var(--color-text-muted);
    font-size: 10px;
    flex-shrink: 0;
  }
  .outline-empty {
    padding: 12px 10px;
    font-size: 11px;
    color: var(--color-text-muted);
    font-style: italic;
  }

  /* API Reference */
  .api-toggle {
    color: var(--color-text-muted);
    font-size: 10px;
  }
  .api-ref-body {
    padding: 4px 0;
    overflow-y: auto;
    flex: 1;
  }
  .api-group {
    margin-bottom: 2px;
  }
  .api-group-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px;
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-secondary);
    cursor: pointer;
    user-select: none;
  }
  .api-group-title:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .api-count {
    font-size: 9px;
    color: var(--color-text-muted);
    font-weight: 400;
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 5px;
    border-radius: 8px;
  }
  .api-items {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    padding: 4px 10px 6px;
  }
  .api-fn {
    font-size: 10px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    padding: 2px 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.06);
    color: var(--color-text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }
  .api-fn:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .outline-header-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .outline-action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    background: transparent;
    border: none;
    color: var(--color-text-muted);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    border-radius: var(--radius-sm);
    padding: 0;
    line-height: 1;
  }
  .outline-action-btn:hover {
    color: var(--color-text);
    background: var(--color-bg-surface);
  }
  .outline-section-header {
    display: flex;
    align-items: center;
    padding: 6px 10px 4px;
    flex-shrink: 0;
  }
  .outline-divider {
    height: 1px;
    background: var(--color-border);
    margin: 2px 8px;
    flex-shrink: 0;
  }
  .outline-files {
    max-height: 200px;
    overflow-y: auto;
    padding: 2px 0;
    flex-shrink: 0;
  }
  .file-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    background: transparent;
    border: none;
    color: var(--color-text-secondary);
    font-size: 12px;
    font-family: 'JetBrains Mono', monospace;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }
  .file-item:hover {
    background: var(--color-bg-surface);
    color: var(--color-text-primary);
  }
  .file-item.active {
    background: var(--color-bg-surface);
    color: var(--color-text-primary);
  }
  .file-icon {
    color: var(--color-accent);
    font-size: 12px;
    font-weight: 600;
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }
  .file-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-item.renaming {
    padding: 3px 6px;
  }
  .file-rename-input {
    width: 100%;
    padding: 2px 4px;
    font-size: 12px;
    font-family: 'JetBrains Mono', monospace;
    background: var(--color-bg);
    border: 1px solid var(--color-accent);
    border-radius: 3px;
    color: var(--color-text-primary);
    outline: none;
  }
  /* Context Menu */
  .ctx-overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
  }
  .ctx-menu {
    position: fixed;
    z-index: 201;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 4px 0;
    min-width: 130px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }
  .ctx-item {
    display: block;
    width: 100%;
    padding: 6px 12px;
    background: transparent;
    border: none;
    color: var(--color-text-secondary);
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .ctx-item:hover {
    background: var(--color-bg-surface);
    color: var(--color-text-primary);
  }
  .ctx-danger {
    color: #f87171;
  }
  .ctx-danger:hover {
    background: rgba(248, 113, 113, 0.1);
    color: #fca5a5;
  }
  .ctx-divider {
    height: 1px;
    background: var(--color-border);
    margin: 4px 0;
  }
  .outline-collapsed-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    flex-shrink: 0;
    background: var(--color-bg-elevated);
    border: none;
    border-right: 1px solid var(--color-border);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .outline-collapsed-btn:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }

  @media (max-width: 767px) {
    .outline-panel {
      display: none;
    }
    .outline-collapsed-btn {
      display: none;
    }
  }

  /* Resizer drag functionality */
  .resizer {
    user-select: none;
  }
  .resizer:active {
    background: var(--color-accent);
  }
  
  /* Learn Overlay */
  .learn-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 150;
    background: var(--color-bg);
    display: flex;
    flex-direction: column;
    animation: fadeIn 0.15s ease;
  }
  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  .learn-overlay-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 20px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .learn-overlay-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--color-text-primary);
  }
  .learn-overlay-close {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 8px;
    background: transparent;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    border-radius: var(--radius-md);
    transition: background 0.15s, color 0.15s;
  }
  .learn-overlay-close:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }
  
  /* Hide workbench when learn is open (but keep mounted) */
  .workbench-hidden {
    display: none !important;
  }

  /* Learn Panel */
  .learn-panel {
    background: var(--color-bg-surface);
    flex: 1;
    overflow-y: auto;
  }
  .learn-content {
    max-width: 900px;
    margin: 0 auto;
    padding: var(--space-lg);
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-lg);
  }
  .learn-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .learn-section h3 {
    font-size: 14px;
    font-weight: 600;
    color: var(--color-accent);
    margin: 0;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .learn-section p {
    font-size: 13px;
    color: var(--color-text-secondary);
    margin: 0;
    line-height: 1.5;
  }
  .learn-section code {
    background: var(--color-bg-elevated);
    padding: 2px 6px;
    border-radius: 4px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
  }
  .learn-code {
    background: var(--color-bg-elevated);
    padding: 12px;
    border-radius: var(--radius-md);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    color: var(--color-text-primary);
    overflow-x: auto;
    margin: 0;
    line-height: 1.6;
  }
  .learn-modes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .learn-mode {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .learn-mode strong {
    color: var(--color-text-primary);
    font-size: 13px;
  }
  .learn-mode span {
    color: var(--color-text-muted);
    font-size: 12px;
  }
  .learn-functions {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .learn-fn-group {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: baseline;
  }
  .learn-fn-group strong {
    min-width: 80px;
    font-size: 12px;
    color: var(--color-text-secondary);
  }
  .learn-fn-group code {
    font-size: 11px;
  }
  
  /* Scoped styles for additions */
  .repl-toggle {
    margin-left: auto;
    border-color: var(--color-accent) !important;
  }
  .repl-toggle.active {
    background: var(--color-accent-subtle);
    color: var(--color-accent);
  }
  .examples-scroll {
    overflow-y: auto;
  }
  .output-pane.collapsed .output-body {
    display: none;
  }
  .header-run-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--color-accent);
    color: white;
    border: none;
    border-radius: var(--radius-md);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s, transform 0.1s;
  }
  .header-run-btn:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }
  .header-run-btn:active:not(:disabled) {
    transform: scale(0.98);
  }
  .header-run-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .header-run-btn .spinner-icon {
    animation: spin 1s linear infinite;
  }
  .header-test-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: transparent;
    color: var(--color-accent);
    border: 1px solid var(--color-accent);
    border-radius: var(--radius-md);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .header-test-btn:hover:not(:disabled) {
    background: var(--color-accent);
    color: white;
  }
  .header-test-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .header-test-btn .spinner-icon {
    animation: spin 1s linear infinite;
  }
  .output-pane.collapsed {
    min-height: auto;
  }
  .collapse-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 0.15s;
  }
  .collapse-toggle:hover {
    color: var(--color-text);
  }
  .collapse-toggle :global(svg) {
    flex-shrink: 0;
  }
  .output-fab {
    position: fixed;
    bottom: 16px;
    right: 16px;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: var(--color-accent);
    color: white;
    border: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    transition: transform 0.15s, box-shadow 0.15s;
    z-index: 100;
  }
  .output-fab:hover {
    transform: scale(1.1);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }
  .mobile-toggle-bar {
    display: none;
  }
  .mobile-toggle-btn {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 12px 16px;
    background: var(--color-bg-elevated);
    border: none;
    border-top: 1px solid var(--color-border);
    color: var(--color-text-muted);
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    transition: background 0.15s, color 0.15s;
  }
  .mobile-toggle-btn:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }
  .mobile-toggle-btn .toggle-label {
    color: var(--color-text-secondary);
  }
  .spacer {
    flex: 0 1 0%;
    min-height: 0;
  }
  .mobile-menu-btn {
    display: flex;
    padding: 8px;
    background: transparent;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background 0.15s, color 0.15s;
  }
  .mobile-menu-btn:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }
  .drawer-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 200;
  }
  .drawer {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 280px;
    max-width: 80vw;
    background: var(--color-bg-elevated);
    border-right: 1px solid var(--color-border);
    z-index: 201;
    display: flex;
    flex-direction: column;
    animation: slideIn 0.2s ease;
  }
  @keyframes slideIn {
    from { transform: translateX(-100%); }
    to { transform: translateX(0); }
  }
  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md) var(--space-lg);
    border-bottom: 1px solid var(--color-border);
    font-weight: 600;
    font-size: 16px;
  }
  .drawer-close {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    background: transparent;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background 0.15s, color 0.15s;
  }
  .drawer-close:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }
  .drawer-content {
    padding: var(--space-sm);
    overflow-y: auto;
    flex: 1;
  }
  .drawer-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 12px var(--space-md);
    background: transparent;
    border: none;
    border-radius: var(--radius-md);
    color: var(--color-text-secondary);
    font-size: 14px;
    text-align: left;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .drawer-item:hover {
    background: var(--color-bg-surface);
    color: var(--color-text);
  }
  .drawer-item.active {
    background: var(--color-accent-subtle);
    color: var(--color-accent);
  }
  .drawer-divider {
    height: 1px;
    background: var(--color-border);
    margin: var(--space-sm) var(--space-md);
  }
  .output-section {
    height: 100%;
    overflow-y: auto;
  }
  .editor-pane {
    overflow-y: auto;
  }
  .editor-pane.full-height {
    height: 100%;
  }
  .editor-pane.full-height .editor-section {
    height: 100%;
  }
  .editor-pane.full-height .editor-wrapper {
    height: 100%;
  }

  /* Mobile styles */
  @media (max-width: 767px) {
    .header-title {
      display: none;
    }
    .pill-label {
      display: none;
    }
    .header-toggle {
      display: none;
    }
    .header-icon-btn {
      display: none;
    }
    .network-badge {
      font-size: 11px;
      padding: 4px 6px;
    }
    .wallet-btn {
      display: none;
    }
    .pill-tab {
      padding: 6px 10px;
    }
    .header {
      gap: 8px;
      padding: 6px 8px;
    }
    .mobile-toggle-btn {
      display: none;
    }
    .mobile-toggle-bar {
      display: block;
      padding: 8px;
      background: var(--color-bg);
      border-top: 1px solid var(--color-border);
    }
  }

  /* Test Results */
  .test-results-panel {
    margin: var(--space-md) 0;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
  }
  .test-results-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    background: var(--color-bg);
    border-bottom: 1px solid var(--color-border);
  }
  .test-results-icon {
    font-size: 16px;
    font-weight: 600;
  }
  .test-results-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text);
  }
  .test-count-failed {
    font-size: 12px;
    color: #ff6b6b;
    margin-left: auto;
  }
  .test-results-list {
    padding: 4px 0;
  }
  .test-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 14px;
    font-size: 13px;
    border-bottom: 1px solid var(--color-border);
  }
  .test-item:last-child {
    border-bottom: none;
  }
  .test-item.passed {
    color: var(--color-text-secondary);
  }
  .test-item.failed {
    background: rgba(255, 107, 107, 0.1);
  }
  .test-status-icon {
    font-size: 14px;
    flex-shrink: 0;
  }
  .test-item.passed .test-status-icon {
    color: var(--color-accent);
  }
  .test-item.failed .test-status-icon {
    color: #ff6b6b;
  }
  .test-name {
    color: var(--color-text);
    font-weight: 500;
  }
  .test-error {
    color: #ff6b6b;
    font-size: 12px;
    margin-left: auto;
    opacity: 0.9;
  }

  /* NEAR Storage Inspector */
  .near-storage-panel {
    margin: var(--space-md) 0;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
  }
  .near-storage-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    background: var(--color-bg);
    border-bottom: 1px solid var(--color-border);
  }
  .near-storage-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-secondary);
  }
  .near-storage-actions {
    display: flex;
    gap: 6px;
  }
  .near-storage-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-bg-surface);
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .near-storage-btn:hover {
    background: var(--color-bg);
    color: var(--color-text);
    border-color: var(--color-accent);
  }
  .near-storage-btn-danger:hover {
    background: rgba(255, 107, 107, 0.1);
    color: #ff6b6b;
    border-color: #ff6b6b;
  }
  .near-storage-content {
    padding: 8px 0;
  }
  .near-storage-empty {
    padding: 12px 14px;
    font-size: 12px;
    color: var(--color-text-muted);
    font-style: italic;
  }
  .near-storage-entry {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--color-border);
    font-size: 13px;
  }
  .near-storage-entry:last-child {
    border-bottom: none;
  }
  .near-storage-key {
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: 12px;
    min-width: 80px;
    word-break: break-all;
  }
  .near-storage-value {
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: 12px;
    word-break: break-all;
  }

  /* NEAR Method Runner */
  .near-method-panel {
    margin: var(--space-md) 0;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
  }
  .near-method-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    background: var(--color-bg);
    border-bottom: 1px solid var(--color-border);
  }
  .near-method-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-secondary);
  }
  .near-context-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-bg-surface);
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .near-context-toggle:hover {
    background: var(--color-bg);
    color: var(--color-text);
    border-color: var(--color-accent);
  }
  .near-context-form {
    padding: 10px 14px;
    background: var(--color-bg);
    border-bottom: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .near-ctx-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .near-ctx-row label {
    font-size: 12px;
    color: var(--color-text-muted);
    min-width: 56px;
  }
  .near-ctx-row input {
    flex: 1;
    padding: 4px 8px;
    font-size: 12px;
    font-family: var(--font-mono);
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    outline: none;
    transition: border-color 0.15s;
  }
  .near-ctx-row input:focus {
    border-color: var(--color-accent);
  }
  .near-ctx-unit {
    font-size: 11px;
    color: var(--color-text-muted);
  }
  .near-method-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
  }
  .near-method-select {
    flex: 1;
    padding: 6px 10px;
    font-size: 13px;
    font-family: var(--font-mono);
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    cursor: pointer;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 8px center;
    padding-right: 28px;
  }
  .near-method-select:focus {
    border-color: var(--color-accent);
  }
  .near-method-run {
    padding: 6px 16px;
    font-size: 13px;
    font-weight: 500;
    background: var(--color-accent);
    color: #000;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: opacity 0.15s;
  }
  .near-method-run:hover {
    opacity: 0.85;
  }
  .near-method-run:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .near-result-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 14px;
    border-top: 1px solid var(--color-border);
    font-size: 12px;
    font-family: var(--font-mono);
  }
  .near-result-ret {
    color: var(--color-accent);
  }
  .near-result-gas {
    color: var(--color-text-muted);
    font-size: 11px;
  }

  /* Input textarea for method args */
  .near-input-area {
    padding: 0 14px 8px;
  }
  .near-input-textarea {
    width: 100%;
    padding: 8px 10px;
    font-size: 12px;
    font-family: var(--font-mono);
    background: var(--color-bg);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    outline: none;
    resize: vertical;
    line-height: 1.5;
    transition: border-color 0.15s;
    min-height: 44px;
  }
  .near-input-textarea:focus {
    border-color: var(--color-accent);
  }
  .near-input-textarea::placeholder {
    color: var(--color-text-muted);
    opacity: 0.5;
  }

  /* Panic */
  .near-panic {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 14px;
    background: rgba(255, 80, 80, 0.08);
    border-top: 1px solid rgba(255, 80, 80, 0.2);
  }
  .near-panic-icon {
    color: #ff5050;
    font-size: 14px;
    flex-shrink: 0;
    padding-top: 1px;
  }
  .near-panic-msg {
    color: #ff6b6b;
    font-size: 12px;
    font-family: var(--font-mono);
    word-break: break-all;
    line-height: 1.5;
  }

  /* Logs */
  .near-logs-section {
    padding: 6px 14px 8px;
    border-top: 1px solid var(--color-border);
  }
  .near-logs-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 4px;
  }
  .near-log-entry {
    font-size: 12px;
    font-family: var(--font-mono);
    color: var(--color-text-secondary);
    line-height: 1.6;
    word-break: break-all;
  }
  .near-log-prefix {
    color: var(--color-text-muted);
    font-size: 11px;
    margin-right: 4px;
  }

  /* Storage Diff */
  .near-diff-section {
    padding: 6px 14px 8px;
    border-top: 1px solid var(--color-border);
  }
  .near-diff-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 4px;
  }
  .near-diff-entry {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
    font-family: var(--font-mono);
    line-height: 1.6;
    word-break: break-all;
  }
  .near-diff-key {
    color: var(--color-accent);
    min-width: 40px;
  }
  .near-diff-arrow {
    color: var(--color-text-muted);
  }
  .near-diff-old {
    color: var(--color-text-muted);
    text-decoration: line-through;
    opacity: 0.7;
  }
  .near-diff-slash {
    color: var(--color-text-muted);
    opacity: 0.4;
  }
  .near-diff-new {
    color: var(--color-text);
  }

  /* Receipts (Promise DAG) */
  .near-receipts-section {
    padding: 6px 14px 8px;
    border-top: 1px solid var(--color-border);
  }
  .near-receipts-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 4px;
  }
  .near-receipt-entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 0;
    font-size: 12px;
    font-family: var(--font-mono);
    line-height: 1.5;
  }
  .near-receipt-method {
    color: var(--color-accent);
    font-weight: 500;
  }
  .near-receipt-meta {
    color: var(--color-text-muted);
    font-size: 11px;
  }
  .near-receipt-result {
    color: var(--color-text-secondary);
    font-size: 11px;
    word-break: break-all;
  }
</style>