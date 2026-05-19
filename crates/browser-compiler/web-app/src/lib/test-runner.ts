/**
 * Test runner for Lisp code.
 * Parses (test "name" body...) forms and runs them.
 */

export interface TestResult {
  name: string;
  passed: boolean;
  error?: string;
  output?: string;
}

export interface TestRunResult {
  tests: TestResult[];
  passed: number;
  failed: number;
  total: number;
}

/**
 * Assert helper functions to prepend to test code.
 * These provide assertion primitives for tests.
 */

const ASSERT_HELPERS = `
;; Assertion helpers - cause WASM trap on failure
(define (assert-equal expected actual)
  (if (= expected actual)
      expected
      (abort)))

(define (assert-true expr)
  (if expr true (abort)))

(define (assert-false expr)
  (if (not expr) false (abort)))
`;

/**
 * Parse source code into test definitions and setup code.
 * Test format: (test "name" body...)
 */
export function parseTests(source: string): { setupCode: string; tests: { name: string; body: string }[] } {
  const tests: { name: string; body: string }[] = [];
  const setupLines: string[] = [];
  const lines = source.split('\n');
  
  let inTest = false;
  let parenDepth = 0;
  let currentTest: { name: string; bodyLines: string[] } | null = null;
  let i = 0;
  
  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();
    
    // Skip empty lines and comments outside tests
    if (!inTest && (trimmed === '' || trimmed.startsWith(';'))) {
      setupLines.push(line);
      i++;
      continue;
    }
    
    // Check for test definition start
    if (trimmed.startsWith('(test "') || trimmed.startsWith('(test \'')) {
      inTest = true;
      parenDepth = 0;
      
      // Extract test name
      const nameMatch = trimmed.match(/^\(test\s+"([^"]+)"/);
      if (nameMatch) {
        currentTest = { name: nameMatch[1], bodyLines: [] };
        parenDepth = (trimmed.match(/\(/g) || []).length - (trimmed.match(/\)/g) || []).length;
        
        // Handle single-line test
        if (parenDepth === 0) {
          // Extract body: (test "name" body)
          const bodyStart = trimmed.indexOf(nameMatch[1]) + nameMatch[1].length + 1;
          const body = trimmed.slice(bodyStart).trim().replace(/\)$/, '');
          if (body) {
            tests.push({ name: currentTest.name, body });
          }
          inTest = false;
          currentTest = null;
        }
      }
      i++;
      continue;
    }
    
    if (inTest && currentTest) {
      currentTest.bodyLines.push(line);
      parenDepth += (line.match(/\(/g) || []).length;
      parenDepth -= (line.match(/\)/g) || []).length;
      
      if (parenDepth <= 0) {
        // Test complete - remove trailing closing paren
        const body = currentTest.bodyLines.join('\n')
          .trim()
          .replace(/\)$/, ''); // Remove trailing ) from test
        tests.push({
          name: currentTest.name,
          body: body.trim()
        });
        inTest = false;
        currentTest = null;
      }
    } else {
      setupLines.push(line);
    }
    
    i++;
  }
  
  // Filter out test definitions from setup code
  const setupCode = setupLines.join('\n');
  
  return { setupCode, tests };
}

/**
 * Build test runner code that executes one test and returns result.
 */
export function buildTestCode(setupCode: string, testBody: string): string {
  return `${ASSERT_HELPERS}
${setupCode}

(define (run)
  ${testBody})
`;
}