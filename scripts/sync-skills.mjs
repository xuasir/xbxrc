import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import process from "node:process";

const ROOT = resolve(dirname(new URL(import.meta.url).pathname), "..");
const LOCK_PATH = resolve(ROOT, "skills-lock.json");
const LOCAL_SKILLS_ROOT = resolve(ROOT, ".agents", "skills");

function readLock() {
  const raw = readFileSync(LOCK_PATH, "utf8");
  return JSON.parse(raw);
}

function writeLock(lock) {
  writeFileSync(LOCK_PATH, `${JSON.stringify(lock, null, 2)}\n`, "utf8");
}

function walkFiles(root) {
  const files = [];
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    const entries = readdirSync(current, { withFileTypes: true });
    for (const entry of entries) {
      const abs = join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(abs);
        continue;
      }
      if (entry.isFile()) {
        files.push(abs);
      }
    }
  }
  files.sort((a, b) => a.localeCompare(b));
  return files;
}

function dirHash(dir) {
  const h = createHash("sha256");
  for (const file of walkFiles(dir)) {
    const rel = relative(dir, file).replaceAll("\\", "/");
    h.update(rel);
    h.update("\n");
    h.update(readFileSync(file));
    h.update("\n");
  }
  return h.digest("hex");
}

function ensureDir(dir) {
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
}

function syncOneSkillFromRepo(repoRoot, skillName) {
  const candidates = [
    resolve(repoRoot, "skills", skillName),
    resolve(repoRoot, ".agents", "skills", skillName),
    resolve(repoRoot, skillName),
  ];
  const source = candidates.find(path => existsSync(path) && statSync(path).isDirectory());
  if (!source) {
    throw new Error(`skill source not found for ${skillName} under ${repoRoot}`);
  }
  ensureDir(LOCAL_SKILLS_ROOT);
  const target = resolve(LOCAL_SKILLS_ROOT, skillName);
  rmSync(target, { recursive: true, force: true });
  cpSync(source, target, { recursive: true });
  return target;
}

function parseArgs(argv) {
  const [cmd = "verify", ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 1) {
    const token = rest[i];
    if (token === "--from-repo") {
      options.fromRepo = rest[i + 1];
      i += 1;
      continue;
    }
  }
  return { cmd, options };
}

function runVerify(lock) {
  let failed = false;
  for (const [name, spec] of Object.entries(lock.skills ?? {})) {
    const localPath = resolve(LOCAL_SKILLS_ROOT, name);
    if (!existsSync(localPath)) {
      failed = true;
      console.error(`[skills] missing: ${name} (${localPath})`);
      continue;
    }
    const hash = dirHash(localPath);
    if (hash !== spec.computedHash) {
      failed = true;
      console.error(
        `[skills] drift: ${name} expected=${spec.computedHash} actual=${hash}`,
      );
      continue;
    }
    console.log(`[skills] ok: ${name}`);
  }
  if (failed) {
    process.exitCode = 1;
  }
}

function runLock(lock) {
  lock.version = 2;
  lock.updatedAt = new Date().toISOString();
  for (const [name, spec] of Object.entries(lock.skills ?? {})) {
    const localPath = resolve(LOCAL_SKILLS_ROOT, name);
    if (!existsSync(localPath)) {
      console.warn(`[skills] skip lock (missing local): ${name}`);
      continue;
    }
    spec.computedHash = dirHash(localPath);
    spec.localPath = `.agents/skills/${name}`;
    spec.lastLockedAt = lock.updatedAt;
    console.log(`[skills] locked: ${name} ${spec.computedHash}`);
  }
  writeLock(lock);
}

function runSync(lock, options) {
  const fromRepo = options.fromRepo ? resolve(options.fromRepo) : null;
  if (!fromRepo) {
    throw new Error("sync requires --from-repo <path>");
  }
  for (const [name] of Object.entries(lock.skills ?? {})) {
    const target = syncOneSkillFromRepo(fromRepo, name);
    console.log(`[skills] synced: ${name} -> ${target}`);
  }
  runLock(lock);
}

function main() {
  const lock = readLock();
  const { cmd, options } = parseArgs(process.argv.slice(2));
  if (cmd === "verify") {
    runVerify(lock);
    return;
  }
  if (cmd === "lock") {
    runLock(lock);
    return;
  }
  if (cmd === "sync") {
    runSync(lock, options);
    return;
  }
  throw new Error(`unsupported command: ${cmd}`);
}

main();
