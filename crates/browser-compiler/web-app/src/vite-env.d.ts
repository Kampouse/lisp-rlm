/// <reference types="vite/client" />

// Injected by the build-info-inject plugin in vite.config.ts
declare const __BUILD_INFO__: { rev: string; dirty: boolean; time: string };

// Monaco 0.55 ships these subpaths without type declarations — the package
// `exports` map maps "./*" to raw runtime files (its .d.ts files are stubs).
// Runtime-verified shapes only: side-effect imports return nothing, and the
// TS-language contribution re-exports the LanguageServiceDefaults the API
// namespace deprecates (`{ deprecated: true }` stub).
declare module 'monaco-editor/esm/vs/editor/editor.all';
declare module 'monaco-editor/esm/vs/basic-languages/typescript/typescript.contribution';
declare module 'monaco-editor/esm/vs/language/typescript/monaco.contribution' {
  export const typescriptDefaults: {
    addExtraLib(content: string, filePath?: string): IDisposable;
    setDiagnosticsOptions(options: {
      noSemanticValidation?: boolean;
      noSyntaxValidation?: boolean;
      diagnosticCodesToIgnore?: number[];
    }): void;
  };
  export const javascriptDefaults: unknown;
}