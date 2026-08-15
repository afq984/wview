import { describe, expect, it } from 'vitest';
import { fmtAll, fmtOne } from './comments';

describe('comment export format', () => {
  it('formats a single-line comment per spec', () => {
    expect(
      fmtOne('src/main.rs', {
        start: 32,
        end: 32,
        quoted: ['def foo():'],
        body: 'please add a docstring',
      }),
    ).toBe('src/main.rs:32:\n> def foo():\nplease add a docstring');
  });

  it('quotes every line of a multi-line selection', () => {
    expect(
      fmtOne('a.py', { start: 10, end: 11, quoted: ['fn a() {', '}'], body: 'why' }),
    ).toBe('a.py:10:\n> fn a() {\n> }\nwhy');
  });

  it('joins batches with blank lines, sorted by path then line', () => {
    const out = fmtAll([
      {
        path: 'b.rs',
        comments: [
          { start: 5, end: 5, quoted: ['x'], body: 'later' },
          { start: 1, end: 1, quoted: ['y'], body: 'first' },
        ],
      },
    ]);
    expect(out).toBe('b.rs:1:\n> y\nfirst\n\nb.rs:5:\n> x\nlater');
  });
});
