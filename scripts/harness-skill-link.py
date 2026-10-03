#!/usr/bin/env python3
# ECL-HARNESS-CONNECTOR
"""Attach or detach this worktree's project-local shared Harness Skill."""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
from pathlib import Path


SKILL_NAME = "beaver-4a20fc43577a-harness"
PROJECT_ID = "beaver-4a20fc43577a"


def git(*args: str, cwd: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(cwd), *args],
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git command failed")
    return result.stdout.strip()


def same_target(path: Path, target: Path) -> bool:
    try:
        return path.resolve() == target.resolve()
    except OSError:
        return False


def is_link_like(path: Path) -> bool:
    is_junction = getattr(os.path, "isjunction", None)
    if path.is_symlink() or bool(is_junction and is_junction(path)):
        return True
    if os.name != "nt":
        return False
    try:
        metadata = os.lstat(path)
    except OSError:
        return False
    reparse_flag = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
    return bool(getattr(metadata, "st_file_attributes", 0) & reparse_flag and path.is_dir())


def reject_linked_ancestors(root: Path, path: Path) -> None:
    root = Path(os.path.abspath(root))
    parent = Path(os.path.abspath(path.parent))
    try:
        relative = parent.relative_to(root)
    except ValueError as exc:
        raise RuntimeError(f"Skill path escapes this worktree: {path}") from exc
    current = root
    for part in relative.parts:
        current = current / part
        if (current.exists() or current.is_symlink()) and is_link_like(current):
            raise RuntimeError(f"Skill path must not traverse a link or junction: {current}")


def link_directory(root: Path, path: Path, target: Path) -> str:
    if Path(os.path.abspath(path)) == Path(os.path.abspath(target)):
        return "physical"
    reject_linked_ancestors(root, path)
    if path.exists() or path.is_symlink():
        if same_target(path, target):
            return "existing"
        raise RuntimeError(f"Skill path collision: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        result = subprocess.run(
            ["cmd", "/c", "mklink", "/J", str(path), str(target)],
            text=True,
            encoding="utf-8",
            errors="replace",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or f"could not create junction {path}")
    else:
        path.symlink_to(os.path.relpath(target, path.parent), target_is_directory=True)
    return "attached"


def remove_created_link(path: Path) -> None:
    if path.is_symlink():
        path.unlink()
    elif path.exists() or is_link_like(path):
        os.rmdir(path)


def ensure_local_skill_excludes(common: Path) -> None:
    exclude = common / "info" / "exclude"
    existing = exclude.read_text(encoding="utf-8") if exclude.exists() else ""
    lines = existing.splitlines()
    wanted = [
        f"/.agents/skills/{SKILL_NAME}",
        f"/.claude/skills/{SKILL_NAME}",
    ]
    if all(value in lines for value in wanted):
        return
    exclude.parent.mkdir(parents=True, exist_ok=True)
    for value in wanted:
        if value not in lines:
            lines.append(value)
    exclude.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")


def main() -> int:
    arguments = sys.argv[1:]
    if any(argument != "--detach" for argument in arguments) or arguments.count("--detach") > 1:
        raise RuntimeError("usage: harness-skill-link.py [--detach]")
    detach = "--detach" in arguments
    current = Path.cwd().resolve()
    root = Path(git("rev-parse", "--show-toplevel", cwd=current)).resolve()
    common_raw = git("rev-parse", "--git-common-dir", cwd=root)
    common = Path(common_raw)
    if not common.is_absolute():
        common = (root / common).resolve()
    worktrees = git("worktree", "list", "--porcelain", cwd=root).splitlines()
    primary_line = next((line for line in worktrees if line.startswith("worktree ")), None)
    if common.name == ".git" and common.parent.is_dir():
        primary = common.parent.resolve()
    elif primary_line:
        primary = Path(primary_line[len("worktree "):].strip()).resolve()
    else:
        raise RuntimeError("could not resolve the primary worktree")
    canonical = primary / ".agents" / "skills" / SKILL_NAME
    links = {
        "codex": root / ".agents" / "skills" / SKILL_NAME,
        "claude": root / ".claude" / "skills" / SKILL_NAME,
    }
    if detach:
        if root == primary:
            raise RuntimeError("the primary worktree hosts the physical project Harness and cannot be detached")
        result = {}
        for name, path in links.items():
            if not os.path.lexists(path):
                result[name] = {"path": str(path), "status": "missing"}
            elif not is_link_like(path):
                raise RuntimeError(f"refusing to detach an unmanaged physical Skill path: {path}")
            elif not same_target(path, canonical):
                raise RuntimeError(f"refusing to detach a Skill link with the wrong target: {path}")
            else:
                result[name] = {"path": str(path), "status": "detached"}
        removed = []
        try:
            for name, path in links.items():
                if result[name]["status"] == "detached":
                    remove_created_link(path)
                    removed.append(path)
        except Exception as exc:
            rollback_errors = []
            for path in reversed(removed):
                try:
                    link_directory(root, path, canonical)
                except Exception as rollback_exc:
                    rollback_errors.append(f"{path}: {rollback_exc}")
            detail = f"; rollback failed for {', '.join(rollback_errors)}" if rollback_errors else ""
            raise RuntimeError(f"could not detach all shared Harness links: {exc}{detail}") from exc
        print(json.dumps({"ok": True, "action": "detached", "skill": str(canonical), "links": result}, indent=2))
        return 0
    if is_link_like(canonical):
        raise RuntimeError(f"canonical project Harness must be physical: {canonical}")
    if not (canonical / "SKILL.md").is_file():
        raise RuntimeError(f"canonical project Harness is missing: {canonical}")
    manifest = json.loads((canonical / "state" / "manifest.json").read_text(encoding="utf-8"))
    if (
        manifest.get("project_id") != PROJECT_ID
        or manifest.get("skill_name") != SKILL_NAME
    ):
        raise RuntimeError("canonical project Harness manifest does not match this Git project")
    ensure_local_skill_excludes(common)
    result = {}
    created = []
    try:
        for name, path in links.items():
            status = link_directory(root, path, canonical)
            result[name] = {"path": str(path), "status": status}
            if status == "attached":
                created.append(path)
    except Exception:
        for path in reversed(created):
            remove_created_link(path)
        raise
    print(json.dumps({"ok": True, "action": "attached", "skill": str(canonical), "links": result}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
