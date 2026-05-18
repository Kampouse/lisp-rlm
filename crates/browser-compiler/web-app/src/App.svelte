<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as monaco from 'monaco-editor';
  import { initCompiler, compile, runPure, runWasi, toHexDump, type CompileTarget, type CompileResult } from './lib/compiler.ts';
  import { examples } from './lib/examples.ts';
  import { connectWallet, disconnectWallet, deployP1, deployP2, getWalletState, type WalletState, type DeployResult, type Network } from './lib/wallet.ts';

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
  let walletState: WalletState = $state({ connected: false, accountId: null, network: 'testnet' });
  let activeExample: number = $state(0);
  let editorInstance: monaco.editor.IStandaloneCodeEditor | null = $state(null);
  let editorContainer: HTMLDivElement | null = $state(null);
  let contractName: string = $state('my-contract');
  let network: Network = $state('testnet');
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

  function saveState() {
    try {
      const state = { source, target, activeExample, autoCompile, replMode };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (e) {
      console.warn('Failed to save state:', e);
    }
  }

  function loadState(): { source: string; target: CompileTarget; activeExample: number; autoCompile: boolean; replMode: boolean } | null {
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
      monaco.editor.setModelMarkers(model, 'lisp-rlm', errors);
    }
  }

  function clearMonacoMarkers() {
    const model = editorInstance?.getModel();
    if (model) {
      monaco.editor.setModelMarkers(model, 'lisp-rlm', []);
    }
  }

  // ============================================
  // Monaco setup
  // ============================================
  function setupMonaco() {
    if (!editorContainer) return;

    monaco.languages.register({ id: 'lisp-rlm' });

    monaco.languages.setMonarchTokensProvider('lisp-rlm', {
      ignoreCase: true,
      brackets: [
        { open: '(', close: ')', token: 'delimiter.parenthesis' },
        { open: '[', close: ']', token: 'delimiter.square' },
      ],
      keywords: [
        'define', 'def', 'defn', 'fn', 'lambda', 'let', 'let*', 'if', 'cond',
        'when', 'do', 'begin', 'and', 'or', 'not', 'set!', 'atom', 'car', 'cdr',
        'cons', 'list', 'map', 'filter', 'reduce', 'range', 'str', 'inc', 'dec',
        'http-get', 'true', 'false', 'nil', 'null',
      ],
      operators: ['=', 'not=', '+', '-', '*', '/', '<', '>', '<=', '>='],
      tokenizer: {
        root: [
          { regex: ';.*$', action: { token: 'comment' } },
          { regex: '"', action: { token: 'string', next: '@string' } },
          { regex: '0x[0-9a-fA-F]+', action: { token: 'number' } },
          { regex: '-?[0-9]+\\.?[0-9]*', action: { token: 'number' } },
          { regex: ':[a-zA-Z_\\-][a-zA-Z0-9_\\-]*', action: { token: 'tag' } },
          { regex: '[()\\[\\]]', action: { token: 'delimiter.parenthesis' } },
          {
            regex: '[a-zA-Z_\\-!\\?][a-zA-Z0-9_\\-!\\?]*',
            action: {
              cases: {
                '@keywords': { token: 'keyword' },
                '@operators': { token: 'operator' },
                '@default': { token: 'identifier' },
              },
            },
          },
          { regex: '\\s+', action: { token: 'white' } },
        ],
        string: [
          { regex: '"', action: { token: 'string', next: '@pop' } },
          { regex: '\\\\.', action: { token: 'string.escape' } },
          { regex: '[^"\\\\]+', action: { token: 'string' } },
        ],
      },
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
        { token: 'tag', foreground: 'ffcb6b' },
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
    result = null;
    deployResult = null;
    showDeployPanel = false;
    runResult = null;
    clearMonacoMarkers();
    await new Promise(r => setTimeout(r, 50));
    try {
      result = compile(source, target);
      
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
    if (!result?.success || !result.wasmBytes || running) return;
    if (target === 'p1') {
      runResult = 'ℹ NEAR contracts run on-chain — use ⚡ Deploy to execute';
      return;
    }
    running = true;
    runResult = null;
    try {
      if (target === 'p2') {
        // P2 outputs WASM Components — need jco transpile (adds ~500KB JS runtime)
        if (result.wat?.includes('(component')) {
          runResult = 'ℹ P2 outputs WASM Components.\n\nBrowser execution requires @bytecodealliance/jco transpilation (~500KB JS runtime).\n\nFor now: use ⚡ Deploy to run on OutLayer, or run `jco transpile` locally.';
        } else {
          runResult = await runWasi(result.wasmBytes);
        }
      } else {
        runResult = await runPure(result.wasmBytes);
      }
    } catch (err: unknown) {
      runResult = `Error: ${err instanceof Error ? err.message : String(err)}`;
    } finally {
      running = false;
    }
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
    
    // Feature 8: Load from localStorage
    const storedState = loadState();
    
    // Priority: URL > localStorage > default
    if (urlState.source) {
      source = urlState.source;
      if (urlState.target) target = urlState.target;
      if (editorInstance) editorInstance.setValue(source);
    } else if (storedState) {
      source = storedState.source;
      target = storedState.target;
      activeExample = storedState.activeExample;
      autoCompile = storedState.autoCompile;
      replMode = storedState.replMode;
      if (editorInstance) editorInstance.setValue(source);
    } else {
      source = examples[0].source;
      target = examples[0].target;
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

  <!-- Fixed Header -->
  <header class="header">
    <div class="header-brand">
      <div class="header-logo">λ</div>
      <span class="header-title">Lisp → WASM</span>
    </div>

    <div class="pill-container" role="tablist">
      <button
        class="pill-tab"
        class:active={target === 'pure'}
        role="tab"
        aria-selected={target === 'pure'}
        onclick={() => { target = 'pure'; result = null; deployResult = null; showDeployPanel = false; runResult = null; clearMonacoMarkers(); saveState(); }}
      >
        ▶ <span class="pill-label">Run</span>
      </button>
      <button
        class="pill-tab"
        class:active={target === 'p1'}
        role="tab"
        aria-selected={target === 'p1'}
        onclick={() => { target = 'p1'; result = null; deployResult = null; showDeployPanel = false; runResult = null; clearMonacoMarkers(); saveState(); }}
      >
        P1 <span class="pill-label">NEAR</span>
      </button>
      <button
        class="pill-tab"
        class:active={target === 'p2'}
        role="tab"
        aria-selected={target === 'p2'}
        onclick={() => { target = 'p2'; result = null; deployResult = null; showDeployPanel = false; runResult = null; clearMonacoMarkers(); saveState(); }}
      >
        P2 <span class="pill-label">WASI</span>
      </button>
    </div>

    <!-- Feature 5: Auto-compile toggle & Feature 7: Share -->
    <button
      class="header-toggle"
      class:active={autoCompile}
      onclick={() => { autoCompile = !autoCompile; saveState(); }}
      title="Auto-compile on type (debounced 300ms)"
    >
      <span class="dot"></span>
      Auto
    </button>

    <button
      class="header-icon-btn"
      onclick={copyShareUrl}
      title="Copy shareable URL"
    >
      🔗
    </button>

    <!-- Network toggle -->
    <button
      class="network-badge"
      onclick={() => { network = network === 'testnet' ? 'mainnet' : 'testnet'; }}
      title="Switch network"
    >
      {network === 'testnet' ? '🧪' : '🔴'} {network}
    </button>

    <!-- Wallet button -->
    {#if walletState.connected}
      <button class="wallet-btn connected" onclick={handleDisconnectWallet} title={walletState.accountId ?? ''}>
        <span class="wallet-dot"></span>
        {shortAccountId}
      </button>
    {:else}
      <button class="wallet-btn" onclick={handleConnectWallet}>
        Connect Wallet
      </button>
    {/if}

    <button
      class="header-compile-btn"
      class:compiling={compiling}
      disabled={!wasmReady || compiling}
      onclick={() => handleCompile(false)}
    >
      {#if compiling}
        <span class="spinner"></span>
        Compiling...
      {:else}
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
          <path d="M8 1L14 4.5V11.5L8 15L2 11.5V4.5L8 1Z" stroke="currentColor" stroke-width="1.5" fill="none"/>
          <path d="M6 8L7.5 9.5L10.5 6.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        Compile
      {/if}
    </button>
  </header>

  <!-- Main Content -->
  <main class="main-content">
    <!-- Editor Section -->
    <section class="editor-section">
      <div class="editor-wrapper">
        <div class="editor-container" bind:this={editorContainer}></div>
      </div>

      <!-- Examples + REPL toggle -->
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
          <span class="example-icon">⚡</span>
          REPL
        </button>
      </div>

      <!-- Shortcut hint -->
      <div class="shortcut-hint">
        <kbd>⌘</kbd><kbd>Enter</kbd> to compile
      </div>
    </section>

    <!-- Feature 9: REPL Panel -->
    {#if replMode}
      <section class="output-section repl-mode">
        <div class="repl-panel">
          <div class="repl-output">
            {#each replHistory as entry}
              <div class="repl-line">
                <span class="repl-prompt">&gt;</span>
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
            <span class="repl-prompt">&gt;</span>
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
      <!-- Output Section -->
      <section class="output-section">
        <div class="output-panel" class:animate-slide-up={result}>
          <div class="output-header">
            <span class="output-title">Output</span>
            {#if result}
              <span class="status-badge" class:success={result.success} class:error={!result.success}>
                <span class="status-dot"></span>
                {result.success ? 'Success' : 'Error'}
              </span>
            {/if}
            {#if result?.success}
              <button
                class="deploy-toggle-btn"
                onclick={() => { showDeployPanel = !showDeployPanel; deployResult = null; }}
              >
                ⚡ Deploy
              </button>
              <button
                class="run-toggle-btn"
                onclick={handleRun}
                disabled={running}
              >
                {#if running}
                  <span class="spinner"></span>
                {:else}
                  ▶ Run
                {/if}
              </button>
            {/if}
          </div>

          {#if result}
            <div class="output-body">
              {#if result.success}
                <div class="output-stats" style="margin-bottom: var(--space-md);">
                  <div class="stat">
                    <span class="stat-label">WASM Size</span>
                    <span class="stat-value success">{formatSize(result.size)}</span>
                  </div>
                  <div class="stat">
                    <span class="stat-label">Compile Time</span>
                    <span class="stat-value">{formatTime(result.timeMs)}</span>
                  </div>
                  <div class="stat">
                    <span class="stat-label">Target</span>
                    <span class="stat-value">{target === 'p1' ? 'NEAR' : target === 'p2' ? 'WASI' : 'Pure'}</span>
                  </div>
                  <div class="stat">
                    <span class="stat-label">Bytes</span>
                    <span class="stat-value">{result.size.toLocaleString()}</span>
                  </div>
                </div>

                <!-- Run Result -->
                {#if runResult !== null}
                  <div class="run-result-panel" class:info={runResult.startsWith('ℹ')}>
                    <div class="run-result-header">
                      <span class="run-result-icon">{runResult.startsWith('Error') ? '✗' : '▶'}</span>
                      <span class="run-result-title">Output</span>
                    </div>
                    <div class="run-result-value" class:error={runResult.startsWith('Error')} class:info-text={runResult.startsWith('ℹ')}>
                      {runResult}
                    </div>
                  </div>
                {/if}

                <!-- Exports -->
                {#if result.exports && result.exports.length > 0}
                  <div class="exports-panel">
                    <span class="exports-label">Exports:</span>
                    {#each result.exports as exp}
                      <span class="export-tag">{exp}</span>
                    {/each}
                  </div>
                {/if}

                <!-- WAT Disassembly -->
                {#if result.wat}
                  <details class="hex-details">
                    <summary class="hex-summary" onclick={() => { showWat = !showWat; }}>
                      {showWat ? '▼' : '▶'} WAT Disassembly
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
                          <div class="deploy-result-text">Contract deployed!</div>
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

                <details class="hex-details">
                  <summary class="hex-summary">WASM Hex Dump</summary>
                  <div class="hex-dump">{hexDump}</div>
                </details>
              {:else}
                <div class="error-message">{result.error}</div>
              {/if}
            </div>
          {:else}
            <div class="empty-output">
              <div class="empty-output-icon">⚡</div>
              <div class="empty-output-text">
                Write Lisp code and hit Compile to generate WASM
              </div>
            </div>
          {/if}
        </div>
      </section>
    {/if}
  </main>

  <footer class="footer">
    Lisp RLM — Write Lisp, Deploy Smart Contracts
  </footer>
</div>

<style>
  /* Scoped styles for additions */
  .repl-toggle {
    margin-left: auto;
    border-color: var(--color-accent) !important;
  }
  .repl-toggle.active {
    background: var(--color-accent-subtle);
    color: var(--color-accent);
  }
  .shortcut-hint {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: var(--space-sm);
    color: var(--color-text-muted);
    font-size: 11px;
  }
  .shortcut-hint kbd {
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    font-family: var(--font-mono);
    font-size: 10px;
  }
</style>