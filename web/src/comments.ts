// Comment export formatting — the first "vanilla core" module extracted from
// the inline JS in src/main.rs (SHARED_JS). Format spec:
//   path:line:
//   > quoted source line(s)
//   comment body

export interface Comment {
  id?: string;
  start: number;
  end: number;
  quoted: string[];
  body: string;
}

export interface FileComments {
  path: string;
  comments: Comment[];
}

export function fmtOne(path: string, c: Comment): string {
  return (
    path + ':' + c.start + ':\n' + c.quoted.map(l => '> ' + l).join('\n') + '\n' + c.body
  );
}

export function fmtAll(entries: FileComments[]): string {
  const parts: string[] = [];
  for (const e of entries) {
    for (const c of [...e.comments].sort((a, b) => a.start - b.start || a.end - b.end)) {
      parts.push(fmtOne(e.path, c));
    }
  }
  return parts.join('\n\n');
}
