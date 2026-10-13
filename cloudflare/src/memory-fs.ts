// In-memory filesystem helper for isomorphic-git inside Cloudflare Workers
// Based on Cloudflare Artifacts isomorphic-git example documentation

interface EntryDir {
  kind: "dir";
  children: Set<string>;
  mtimeMs: number;
}

interface EntryFile {
  kind: "file";
  data: Uint8Array;
  mtimeMs: number;
}

type Entry = EntryDir | EntryFile;

class MemoryStats {
  entry: Entry;

  constructor(entry: Entry) {
    this.entry = entry;
  }

  get size(): number {
    return this.entry.kind === "file" ? this.entry.data.byteLength : 0;
  }

  get mtimeMs(): number {
    return this.entry.mtimeMs;
  }

  get ctimeMs(): number {
    return this.entry.mtimeMs;
  }

  get mode(): number {
    return this.entry.kind === "file" ? 0o100644 : 0o040000;
  }

  isFile(): boolean {
    return this.entry.kind === "file";
  }

  isDirectory(): boolean {
    return this.entry.kind === "dir";
  }

  isSymbolicLink(): boolean {
    return false;
  }
}

export class MemoryFS {
  encoder = new TextEncoder();
  decoder = new TextDecoder();
  entries = new Map<string, Entry>([
    ["/", { kind: "dir", children: new Set(), mtimeMs: Date.now() }],
  ]);

  promises = {
    readFile: this.readFile.bind(this),
    writeFile: this.writeFile.bind(this),
    unlink: this.unlink.bind(this),
    readdir: this.readdir.bind(this),
    mkdir: this.mkdir.bind(this),
    rmdir: this.rmdir.bind(this),
    stat: this.stat.bind(this),
    lstat: this.lstat.bind(this),
  };

  normalize(input: string): string {
    const segments: string[] = [];
    for (const part of input.split("/")) {
      if (!part || part === ".") continue;
      if (part === "..") {
        segments.pop();
        continue;
      }
      segments.push(part);
    }
    return `/${segments.join("/")}` || "/";
  }

  parent(path: string): string {
    const normalized = this.normalize(path);
    if (normalized === "/") return "/";
    const parts = normalized.split("/").filter(Boolean);
    parts.pop();
    return parts.length ? `/${parts.join("/")}` : "/";
  }

  basename(path: string): string {
    return this.normalize(path).split("/").filter(Boolean).pop() ?? "";
  }

  getEntry(path: string): Entry | undefined {
    return this.entries.get(this.normalize(path));
  }

  requireEntry(path: string): Entry {
    const entry = this.getEntry(path);
    if (!entry) throw new Error(`ENOENT: ${path}`);
    return entry;
  }

  requireDir(path: string): EntryDir {
    const entry = this.requireEntry(path);
    if (entry.kind !== "dir") throw new Error(`ENOTDIR: ${path}`);
    return entry;
  }

  async mkdir(path: string, options?: { recursive?: boolean }): Promise<void> {
    const target = this.normalize(path);
    if (target === "/") return;

    const recursive = typeof options === "object" && options !== null && options.recursive;
    const parent = this.parent(target);

    if (!this.entries.has(parent)) {
      if (!recursive) throw new Error(`ENOENT: ${parent}`);
      await this.mkdir(parent, { recursive: true });
    }

    if (this.entries.has(target)) return;

    this.entries.set(target, {
      kind: "dir",
      children: new Set(),
      mtimeMs: Date.now(),
    });

    this.requireDir(parent).children.add(this.basename(target));
  }

  async writeFile(path: string, data: string | Uint8Array): Promise<void> {
    const target = this.normalize(path);
    await this.mkdir(this.parent(target), { recursive: true });

    const bytes = typeof data === "string"
      ? this.encoder.encode(data)
      : data instanceof Uint8Array
        ? data
        : new Uint8Array(data);

    this.entries.set(target, {
      kind: "file",
      data: bytes,
      mtimeMs: Date.now(),
    });

    this.requireDir(this.parent(target)).children.add(this.basename(target));
  }

  async readFile(path: string, options?: { encoding?: string }): Promise<string | Uint8Array> {
    const entry = this.requireEntry(path);
    if (entry.kind !== "file") throw new Error(`EISDIR: ${path}`);
    if (options?.encoding === "utf8") {
      return this.decoder.decode(entry.data);
    }
    return entry.data;
  }

  async unlink(path: string): Promise<void> {
    const target = this.normalize(path);
    const entry = this.requireEntry(target);
    if (entry.kind === "dir") throw new Error(`EISDIR: ${target}`);
    this.entries.delete(target);
    this.requireDir(this.parent(target)).children.delete(this.basename(target));
  }

  async readdir(path: string): Promise<string[]> {
    return Array.from(this.requireDir(path).children);
  }

  async rmdir(path: string): Promise<void> {
    const target = this.normalize(path);
    const dir = this.requireDir(target);
    if (dir.children.size > 0) throw new Error(`ENOTEMPTY: ${target}`);
    this.entries.delete(target);
    this.requireDir(this.parent(target)).children.delete(this.basename(target));
  }

  async stat(path: string): Promise<MemoryStats> {
    return new MemoryStats(this.requireEntry(path));
  }

  async lstat(path: string): Promise<MemoryStats> {
    return this.stat(path);
  }
}
