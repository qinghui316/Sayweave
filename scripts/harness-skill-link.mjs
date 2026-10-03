#!/usr/bin/env node
// ECL-HARNESS-CONNECTOR
// Attach or detach this worktree's project-level shared Harness Skill without requiring Python.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

const skillName = "beaver-4a20fc43577a-harness";
const projectId = "beaver-4a20fc43577a";
const arguments_ = process.argv.slice(2);
if (arguments_.some((argument) => argument !== "--detach") || arguments_.filter((argument) => argument === "--detach").length > 1) {
  throw new Error("usage: harness-skill-link.mjs [--detach]");
}
const detach = arguments_.includes("--detach");

function git(cwd, ...args) {
  return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8" }).trim();
}

function sameTarget(link, target) {
  try {
    return fs.realpathSync.native(link) === fs.realpathSync.native(target);
  } catch {
    return false;
  }
}

function rejectLinkedAncestors(root, link) {
  const relative = path.relative(root, path.dirname(link));
  if (relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error(`Skill path escapes this worktree: ${link}`);
  }
  let current = root;
  for (const part of relative.split(path.sep).filter(Boolean)) {
    current = path.join(current, part);
    if (fs.existsSync(current) && fs.lstatSync(current).isSymbolicLink()) {
      throw new Error(`Skill path must not traverse a link or junction: ${current}`);
    }
  }
}

function addSkillLink(root, link, target) {
  if (path.resolve(link) === path.resolve(target)) return "physical";
  rejectLinkedAncestors(root, link);
  if (fs.existsSync(link)) {
    if (sameTarget(link, target)) return "existing";
    throw new Error(`Skill path collision: ${link}`);
  }
  fs.mkdirSync(path.dirname(link), { recursive: true });
  const type = process.platform === "win32" ? "junction" : "dir";
  const value = process.platform === "win32" ? target : path.relative(path.dirname(link), target);
  fs.symlinkSync(value, link, type);
  return "attached";
}

function ensureLocalSkillExcludes(common) {
  const exclude = path.join(common, "info", "exclude");
  const existing = fs.existsSync(exclude) ? fs.readFileSync(exclude, "utf8") : "";
  const lines = existing.split(/\r?\n/).filter(Boolean);
  const wanted = [
    `/.agents/skills/${skillName}`,
    `/.claude/skills/${skillName}`,
  ];
  if (wanted.every((value) => lines.includes(value))) return;
  fs.mkdirSync(path.dirname(exclude), { recursive: true });
  for (const value of wanted) {
    if (!lines.includes(value)) lines.push(value);
  }
  fs.writeFileSync(exclude, `${lines.join("\n")}\n`, "utf8");
}

const current = process.cwd();
const root = path.resolve(git(current, "rev-parse", "--show-toplevel"));
const commonRaw = git(root, "rev-parse", "--git-common-dir");
const common = path.resolve(root, commonRaw);
const worktreeLines = git(root, "worktree", "list", "--porcelain").split(/\r?\n/);
const firstWorktree = worktreeLines.find((line) => line.startsWith("worktree "));
const primary = path.basename(common) === ".git"
  ? path.dirname(common)
  : path.resolve(firstWorktree?.slice("worktree ".length).trim() ?? "");
if (!primary) throw new Error("could not resolve the primary worktree");
const canonical = path.join(primary, ".agents", "skills", skillName);
const links = {
  codex: path.join(root, ".agents", "skills", skillName),
  claude: path.join(root, ".claude", "skills", skillName),
};
if (detach) {
  if (path.resolve(root) === path.resolve(primary)) {
    throw new Error("the primary worktree hosts the physical project Harness and cannot be detached");
  }
  const result = {};
  for (const [name, link] of Object.entries(links)) {
    const item = fs.lstatSync(link, { throwIfNoEntry: false });
    if (!item) {
      result[name] = { path: link, status: "missing" };
    } else if (!item.isSymbolicLink()) {
      throw new Error(`refusing to detach an unmanaged physical Skill path: ${link}`);
    } else if (!sameTarget(link, canonical)) {
      throw new Error(`refusing to detach a Skill link with the wrong target: ${link}`);
    } else {
      result[name] = { path: link, status: "detached" };
    }
  }
  const removed = [];
  try {
    for (const [name, link] of Object.entries(links)) {
      if (result[name].status === "detached") {
        fs.unlinkSync(link);
        removed.push(link);
      }
    }
  } catch (error) {
    const rollbackErrors = [];
    for (const link of removed.reverse()) {
      try { addSkillLink(root, link, canonical); } catch (rollbackError) {
        rollbackErrors.push(`${link}: ${rollbackError.message}`);
      }
    }
    const detail = rollbackErrors.length ? `; rollback failed for ${rollbackErrors.join(", ")}` : "";
    throw new Error(`could not detach all shared Harness links: ${error.message}${detail}`, { cause: error });
  }
  process.stdout.write(`${JSON.stringify({ ok: true, action: "detached", skill: canonical, links: result }, null, 2)}\n`);
  process.exit(0);
}
const canonicalItem = fs.lstatSync(canonical, { throwIfNoEntry: false });
if (canonicalItem?.isSymbolicLink()) {
  throw new Error(`canonical project Harness must be physical: ${canonical}`);
}
if (!canonicalItem || !fs.statSync(path.join(canonical, "SKILL.md"), { throwIfNoEntry: false })?.isFile()) {
  throw new Error(`canonical project Harness is missing: ${canonical}`);
}
const manifest = JSON.parse(fs.readFileSync(path.join(canonical, "state", "manifest.json"), "utf8"));
if (
  manifest.project_id !== projectId
  || manifest.skill_name !== skillName
) {
  throw new Error("canonical project Harness manifest does not match this Git project");
}
ensureLocalSkillExcludes(common);
const result = {};
const created = [];
try {
  for (const [name, link] of Object.entries(links)) {
    const status = addSkillLink(root, link, canonical);
    result[name] = { path: link, status };
    if (status === "attached") created.push(link);
  }
} catch (error) {
  for (const link of created.reverse()) {
    try { fs.unlinkSync(link); } catch { /* Preserve the original connector error. */ }
  }
  throw error;
}
process.stdout.write(`${JSON.stringify({ ok: true, action: "attached", skill: canonical, links: result }, null, 2)}\n`);
