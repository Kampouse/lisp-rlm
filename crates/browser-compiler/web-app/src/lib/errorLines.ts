// Line-number annotation for compile errors (improvement #2).
//
// The Rust checker's type/scope errors carry no span today (the spanned
// parser is a stub — src/parser.rs:453), so the Monaco lens pins every type
// error to line 1. Full span threading is a parser+AST rewrite; this seam
// gives near-line accuracy browser-side: find the first meaningful
// identifier in the message, then the top-level form containing it.

const NOISE = new Set([
  'type', 'mismatch', 'error', 'undefined', 'variable', 'call', 'expected',
  'function', 'not', 'in', 'scope', 'str', 'int', 'bool', 'list', 'nil',
  'any', 'unknown', 'arity', 'argument', 'arguments', 'requires', 'too',
  'many', 'few', 'line', 'valuestr', 'valueint', 'valuelist', 'valuebool',
  'con', 'arrow', 'tctype', 'string', 'number', 'void', 'the', 'a', 'of',
  'and', 'instead', 'but', 'got',
]);

const WORD = /[A-Za-z_][A-Za-z0-9_-]*/g;

/** Append "(line N)" to a compiler error by locating its first meaningful
 *  identifier in the source. No-op if the error already has a line. */
export function annotateErrorLines(error: string, source: string): string {
  if (!error || /line \d+/i.test(error)) return error;
  const isLisp = /^\s*[;(]/.test(source);
  for (const m of error.matchAll(WORD)) {
    const w = m[0];
    if (NOISE.has(w.toLowerCase())) continue;
    const line = findFormLine(source, w, isLisp);
    if (line > 0) return `${error} (line ${line})`;
  }
  return error;
}

/** 1-based line of the top-level form containing `word`
 *  (whole-token match, comments/strings respected). 0 = not found. */
function findFormLine(source: string, word: string, isLisp: boolean): number {
  const lines = source.split('\n');
  const tok = new RegExp(`(?<![\\w-])${escapeRe(word)}(?![\\w-])`);
  let hit = -1;
  for (let i = 0; i < lines.length; i++) {
    if (tok.test(stripComment(lines[i], isLisp))) {
      hit = i + 1;
      break;
    }
  }
  if (hit < 0) return 0;
  for (const [s, e] of topLevelRanges(lines, isLisp)) {
    if (hit >= s && hit <= e) return s;
  }
  return hit;
}

/** (startLine, endLine) of each depth-0 form (lisp parens / TS braces). */
function topLevelRanges(lines: string[], isLisp: boolean): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  let depth = 0;
  let start = 0;
  let quote: string | null = null;
  for (let i = 0; i < lines.length; i++) {
    const text = stripComment(lines[i], isLisp);
    for (let j = 0; j < text.length; j++) {
      const ch = text[j];
      if (quote) {
        if (ch === '\\') j++;
        else if (ch === quote) quote = null;
        continue;
      }
      // lisp: only " delimits strings (' is symbol quote, ` is syntax quote)
      // ts: " ' ` all open strings/templates
      if (ch === '"' || (!isLisp && (ch === "'" || ch === '`'))) {
        quote = ch;
      } else if (ch === '(' || ch === '{') {
        if (depth === 0) start = i + 1;
        depth++;
      } else if (ch === ')' || ch === '}') {
        if (depth > 0) {
          depth--;
          if (depth === 0 && start > 0) {
            ranges.push([start, i + 1]);
            start = 0;
          }
        }
      }
    }
  }
  return ranges;
}

/** Substring up to the first comment marker outside string literals
 *  (lisp: ';', TS: '//'). */
function stripComment(line: string, isLisp: boolean): string {
  let quote: string | null = null;
  for (let j = 0; j < line.length; j++) {
    const ch = line[j];
    if (quote) {
      if (ch === '\\') j++;
      else if (ch === quote) quote = null;
    } else if (ch === '"' || (!isLisp && (ch === "'" || ch === '`'))) {
      quote = ch;
    } else if (ch === ';' && isLisp) {
      return line.slice(0, j);
    } else if (!isLisp && ch === '/' && line[j + 1] === '/') {
      return line.slice(0, j);
    }
  }
  return line;
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
