// Worker fixture: boots the Bazel-built wview binary against a fresh fixture
// tree on an ephemeral port (port 0) and exposes the parsed base URL.
import { test as base } from '@playwright/test';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export interface WviewServer {
  baseURL: string;
  root: string;
}

export const test = base.extend<{}, { server: WviewServer }>({
  server: [
    async ({}, use) => {
      const root = await mkdtemp(join(process.env.TEST_TMPDIR ?? tmpdir(), 'wview-'));
      await writeFile(join(root, 'f.py'), 'def foo():\n    pass\n');
      await mkdir(join(root, 'sub'), { recursive: true });
      await writeFile(join(root, 'sub', 'lib.rs'), 'fn main() {}\n');
      // Tall file: overflows the viewport for scroll-behavior tests.
      const many = Array.from({ length: 200 }, (_, i) => `x${i + 1} = ${i + 1}`).join('\n');
      await writeFile(join(root, 'many.py'), many + '\n');

      const bin = process.env.WVIEW_BIN ?? join('..', 'bazel-bin', 'wview');
      const proc = spawn(bin, [root, '0'], { stdio: ['ignore', 'pipe', 'inherit'] });
      try {
        const baseURL = await new Promise<string>((resolve, reject) => {
          let buf = '';
          const timer = setTimeout(() => reject(new Error('timeout waiting for wview')), 10_000);
          proc.stdout.on('data', (chunk: Buffer) => {
            buf += chunk.toString();
            const m = buf.match(/ at (http:\/\/[0-9.:]+)\//);
            if (m) {
              clearTimeout(timer);
              resolve(m[1]);
            }
          });
          proc.on('error', (err) => {
            clearTimeout(timer);
            reject(err);
          });
          proc.on('exit', (code) => {
            clearTimeout(timer);
            reject(new Error(`wview exited early with code ${code}`));
          });
        });
        await use({ baseURL, root });
      } finally {
        // The test runs unsandboxed; never leave the server orphaned.
        proc.kill();
      }
    },
    { scope: 'worker' },
  ],
});

export { expect } from '@playwright/test';
