<script lang="ts">
  import { onMount } from 'svelte';
  import * as monaco from 'monaco-editor';
  import { initCompiler, compile, toHexDump, type CompileTarget, type CompileResult } from './lib/compiler.ts';
  import { examples } from './lib/examples.ts';

  // State
  let target: CompileTarget = $state('p1');
  let source: string = $state(examples[0].source);
  let wasmReady: boolean = $state(false);
  let compiling: boolean = $state(false);
  let result: CompileResult | null = $state(null);
  let activeExample: number = $state(0);
  let editorInstance: monaco.editor.IStandaloneCodeEditor | null = $state(null);
  let editorContainer: HTMLDivElement | null = $state(null);

  // Derived
  let hexDump: string = $derived.by(() => {
    if (result && result.success && result.wasmBytes) {
      return toHexDump(result.wasmBytes);
    }
    return '';
  });

  function setupMonaco() {
    if (!editorContainer) return;

    // Define Lisp-like language
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
          // Comments
          { regex: ';.*$', action: { token: 'comment' } },

          // Strings
          { regex: '"', action: { token: 'string', next: '@string' } },

          // Numbers
          { regex: '0x[0-9a-fA-F]+', action: { token: 'number' } },
          { regex: '-?[0-9]+\\.?[0-9]*', action: { token: 'number' } },

          // Keywords with colon prefix
          { regex: ':[a-zA-Z_\\-][a-zA-Z0-9_\\-]*', action: { token: 'tag' } },

          // Parentheses
          { regex: '[()\\[\\]]', action: { token: 'delimiter.parenthesis' } },

          // Identifiers and keywords
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

    // Dark theme for Lisp
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

    // Create editor
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
      scrollbar: {
        verticalScrollbarSize: 6,
        horizontalScrollbarSize: 6,
      },
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: true,
      renderWhitespace: 'none',
      guides: {
        bracketPairs: true,
        indentation: true,
      },
    });

    // Track changes
    editorInstance.onDidChangeModelContent(() => {
      source = editorInstance?.getValue() ?? '';
    });
  }

  function selectExample(index: number) {
    activeExample = index;
    source = examples[index].source;
    target = examples[index].target;
    if (editorInstance) {
      editorInstance.setValue(source);
    }
    result = null;
  }

  async function handleCompile() {
    if (!wasmReady || compiling) return;
    compiling = true;
    result = null;

    // Let UI update before compile
    await new Promise(r => setTimeout(r, 50));

    try {
      result = compile(source, target);
    } finally {
      compiling = false;
    }
  }

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

    return () => {
      editorInstance?.dispose();
    };
  });
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

  <!-- Fixed Header with Pill Tabs -->
  <header class="header">
    <div class="header-brand">
      <div class="header-logo">λ</div>
      <span class="header-title">Lisp → WASM</span>
    </div>

    <div class="pill-container" role="tablist">
      <button
        class="pill-tab"
        class:active={target === 'p1'}
        role="tab"
        aria-selected={target === 'p1'}
        onclick={() => { target = 'p1'; result = null; }}
      >
        P1 <span class="pill-label">NEAR</span>
      </button>
      <button
        class="pill-tab"
        class:active={target === 'p2'}
        role="tab"
        aria-selected={target === 'p2'}
        onclick={() => { target = 'p2'; result = null; }}
      >
        P2 <span class="pill-label">WASI</span>
      </button>
    </div>

    <button
      class="header-compile-btn"
      class:compiling={compiling}
      disabled={!wasmReady || compiling}
      onclick={handleCompile}
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

      <!-- Examples Bar -->
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
      </div>
    </section>

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
                  <span class="stat-value">{target === 'p1' ? 'NEAR' : 'WASI'}</span>
                </div>
                <div class="stat">
                  <span class="stat-label">Bytes</span>
                  <span class="stat-value">{result.size.toLocaleString()}</span>
                </div>
              </div>
              <div class="hex-dump">{hexDump}</div>
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
  </main>

  <!-- Footer -->
  <footer class="footer">
    Lisp RLM — Write Lisp, Deploy Smart Contracts
  </footer>
</div>

<style>
  /* Component-scoped styles are minimal; most are in app.css */
</style>
