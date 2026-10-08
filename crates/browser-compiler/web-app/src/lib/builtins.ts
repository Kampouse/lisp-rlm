
/**
 * Builtin inventory for the lisp-rlm dialect — autocomplete + hover.
 *
 * Single source of truth: the checker's own type env (Rust
 * `TcEnv::builtin_signatures`), shipped through the wasm module as
 * "name<TAB>signature" lines via `publish_builtins()`. The editor can
 * never drift from what the compiler actually accepts — a builtin that
 * shows up here is exactly one the checker will not flag.
 */
let parsed: { name: string; sig: string }[] | null = null;

export function builtinCount(): number {
  return list().length;
}

export function list(): { name: string; sig: string }[] {
  if (parsed) return parsed;
  parsed = [];
  return parsed;
}

export function feed(raw: string): void {
  const items: { name: string; sig: string }[] = [];
  for (const line of raw.split('\n')) {
    const tab = line.indexOf('\t');
    if (tab <= 0) continue;
    const name = line.slice(0, tab);
    const sig = line.slice(tab + 1);
    if (!name || !sig) continue;
    items.push({ name, sig });
  }
  items.sort((a, b) => a.name.localeCompare(b.name));
  parsed = items;
}

export function signatureOf(name: string): string | undefined {
  return list().find(
    (b) => b.name === name || b.name === name.slice(1),
  )?.sig;
}
