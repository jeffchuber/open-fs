#!/usr/bin/env python3
"""
Coding Agent Example

Demonstrates how to use the AX virtual filesystem to build an AI coding agent
with organized storage for context, skills, memories, and code.
"""

import json
import os
from datetime import datetime
from pathlib import Path
from typing import Optional

# Simulated ax import - in real usage this would be the actual ax module
# import ax


class MockVfs:
    """Mock VFS for demonstration when ax module isn't built."""

    def __init__(self, base_path: str):
        self.base = Path(base_path)

    def read_text(self, path: str) -> str:
        full_path = self.base / path.lstrip("/")
        return full_path.read_text()

    def write_text(self, path: str, content: str) -> None:
        full_path = self.base / path.lstrip("/")
        full_path.parent.mkdir(parents=True, exist_ok=True)
        full_path.write_text(content)

    def append_text(self, path: str, content: str) -> None:
        full_path = self.base / path.lstrip("/")
        full_path.parent.mkdir(parents=True, exist_ok=True)
        with open(full_path, "a") as f:
            f.write(content)

    def list(self, path: str) -> list:
        full_path = self.base / path.lstrip("/")
        if not full_path.exists():
            return []

        class Entry:
            def __init__(self, p: Path):
                self.name = p.name
                self.is_dir = p.is_dir()
                self.path = str(p)

        return [Entry(p) for p in full_path.iterdir()]

    def exists(self, path: str) -> bool:
        full_path = self.base / path.lstrip("/")
        return full_path.exists()

    def delete(self, path: str) -> None:
        full_path = self.base / path.lstrip("/")
        if full_path.is_file():
            full_path.unlink()


class CodingAgent:
    """
    An AI coding agent with tiered storage for different types of data.

    Storage Tiers:
    - Hot (context, scratch): Active working data, fast access
    - Warm (skills, memories, code, docs): Knowledge base, indexed
    - Cold (archive): Historical data, read-only
    """

    def __init__(self, vfs):
        self.vfs = vfs

    # =========================================================================
    # CONTEXT MANAGEMENT (Hot Tier)
    # =========================================================================

    def start_task(self, description: str, files: list[str]) -> dict:
        """Initialize context for a new coding task."""
        # Save task description
        self.vfs.write_text("/context/current_task.md", f"# Current Task\n\n{description}")

        # Initialize plan
        plan = """# Execution Plan

## Phase 1: Analysis
- [ ] Understand requirements
- [ ] Review related code

## Phase 2: Implementation
- [ ] Write code
- [ ] Add tests

## Phase 3: Review
- [ ] Self-review
- [ ] Run tests
"""
        self.vfs.write_text("/context/plan.md", plan)

        # Initialize state
        state = {
            "task_id": f"task_{datetime.now().strftime('%Y%m%d_%H%M%S')}",
            "started_at": datetime.now().isoformat(),
            "status": "in_progress",
            "files": files,
            "completed_steps": [],
            "next_step": "Analyze requirements",
        }
        self.vfs.write_text("/context/state.json", json.dumps(state, indent=2))

        return state

    def get_context(self) -> dict:
        """Get the current working context."""
        try:
            return {
                "task": self.vfs.read_text("/context/current_task.md"),
                "plan": self.vfs.read_text("/context/plan.md"),
                "state": json.loads(self.vfs.read_text("/context/state.json")),
            }
        except FileNotFoundError:
            return {"task": None, "plan": None, "state": None}

    def update_state(self, updates: dict) -> dict:
        """Update the current task state."""
        state = json.loads(self.vfs.read_text("/context/state.json"))
        state.update(updates)
        state["updated_at"] = datetime.now().isoformat()
        self.vfs.write_text("/context/state.json", json.dumps(state, indent=2))
        return state

    def complete_step(self, step: str) -> None:
        """Mark a step as completed."""
        state = json.loads(self.vfs.read_text("/context/state.json"))
        if "completed_steps" not in state:
            state["completed_steps"] = []
        state["completed_steps"].append(step)
        state["updated_at"] = datetime.now().isoformat()
        self.vfs.write_text("/context/state.json", json.dumps(state, indent=2))

    # =========================================================================
    # SKILLS MANAGEMENT (Warm Tier)
    # =========================================================================

    def load_skill(self, skill_name: str) -> Optional[str]:
        """Load a skill definition."""
        try:
            return self.vfs.read_text(f"/skills/{skill_name}.md")
        except FileNotFoundError:
            return None

    def save_skill(self, skill_name: str, content: str) -> None:
        """Save a new or updated skill."""
        self.vfs.write_text(f"/skills/{skill_name}.md", content)

    def list_skills(self) -> list[str]:
        """List all available skills."""
        entries = self.vfs.list("/skills")
        return [e.name.replace(".md", "") for e in entries if not e.is_dir and e.name.endswith(".md")]

    def load_tool(self, tool_name: str) -> Optional[dict]:
        """Load a tool definition."""
        try:
            content = self.vfs.read_text(f"/skills/tools/{tool_name}.json")
            return json.loads(content)
        except FileNotFoundError:
            return None

    # =========================================================================
    # MEMORY MANAGEMENT (Warm Tier)
    # =========================================================================

    def record_conversation(self, role: str, content: str) -> None:
        """Record a conversation turn."""
        today = datetime.now().strftime("%Y-%m-%d")
        entry = json.dumps(
            {
                "timestamp": datetime.now().isoformat(),
                "role": role,
                "content": content,
            }
        )
        self.vfs.append_text(f"/memories/conversations/{today}.jsonl", entry + "\n")

    def get_conversations(self, date: Optional[str] = None) -> list[dict]:
        """Get conversations for a specific date or today."""
        date = date or datetime.now().strftime("%Y-%m-%d")
        try:
            content = self.vfs.read_text(f"/memories/conversations/{date}.jsonl")
            return [json.loads(line) for line in content.strip().split("\n") if line]
        except FileNotFoundError:
            return []

    def save_pattern(self, name: str, pattern: str) -> None:
        """Save a learned pattern."""
        self.vfs.write_text(f"/memories/patterns/{name}.md", pattern)

    def load_pattern(self, name: str) -> Optional[str]:
        """Load a pattern by name."""
        try:
            return self.vfs.read_text(f"/memories/patterns/{name}.md")
        except FileNotFoundError:
            return None

    # =========================================================================
    # CODE MANAGEMENT (Warm Tier)
    # =========================================================================

    def save_snippet(self, language: str, name: str, code: str) -> None:
        """Save a code snippet."""
        self.vfs.write_text(f"/code/snippets/{language}/{name}", code)

    def load_snippet(self, language: str, name: str) -> Optional[str]:
        """Load a code snippet."""
        try:
            return self.vfs.read_text(f"/code/snippets/{language}/{name}")
        except FileNotFoundError:
            return None

    def list_snippets(self, language: str) -> list[str]:
        """List snippets for a language."""
        try:
            entries = self.vfs.list(f"/code/snippets/{language}")
            return [e.name for e in entries if not e.is_dir]
        except FileNotFoundError:
            return []

    # =========================================================================
    # SCRATCH WORKSPACE (Hot Tier)
    # =========================================================================

    def save_draft(self, name: str, content: str) -> None:
        """Save a draft to scratch space."""
        self.vfs.write_text(f"/scratch/{name}", content)

    def load_draft(self, name: str) -> Optional[str]:
        """Load a draft from scratch space."""
        try:
            return self.vfs.read_text(f"/scratch/{name}")
        except FileNotFoundError:
            return None

    def clear_scratch(self) -> int:
        """Clear all scratch files. Returns count of files deleted."""
        entries = self.vfs.list("/scratch")
        count = 0
        for entry in entries:
            if not entry.is_dir:
                self.vfs.delete(f"/scratch/{entry.name}")
                count += 1
        return count


def demo():
    """Demonstrate the coding agent capabilities."""
    # Get the directory where this script is located
    script_dir = Path(__file__).parent
    data_dir = script_dir / "data"

    # Create mock VFS pointing to local data
    # In production, use: vfs = ax.load_config_file("ax.yaml")
    vfs = MockVfs(str(data_dir))

    # Mount points map to subdirectories:
    # /context  -> data/local/context
    # /skills   -> data/knowledge/skills
    # /memories -> data/knowledge/memories
    # /code     -> data/knowledge/code
    # /docs     -> data/knowledge/docs
    # /scratch  -> data/local/scratch

    # Create agent with correct path mappings
    class MappedVfs:
        def __init__(self, base):
            self.base = Path(base)
            self.mounts = {
                "/context": "local/context",
                "/scratch": "local/scratch",
                "/skills": "knowledge/skills",
                "/memories": "knowledge/memories",
                "/code": "knowledge/code",
                "/docs": "knowledge/docs",
            }

        def _resolve(self, path: str) -> Path:
            for mount, target in self.mounts.items():
                if path.startswith(mount):
                    return self.base / target / path[len(mount) :].lstrip("/")
            return self.base / path.lstrip("/")

        def read_text(self, path: str) -> str:
            return self._resolve(path).read_text()

        def write_text(self, path: str, content: str) -> None:
            p = self._resolve(path)
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(content)

        def append_text(self, path: str, content: str) -> None:
            p = self._resolve(path)
            p.parent.mkdir(parents=True, exist_ok=True)
            with open(p, "a") as f:
                f.write(content)

        def list(self, path: str) -> list:
            p = self._resolve(path)
            if not p.exists():
                return []

            class Entry:
                def __init__(self, path: Path):
                    self.name = path.name
                    self.is_dir = path.is_dir()

            return [Entry(x) for x in p.iterdir()]

        def exists(self, path: str) -> bool:
            return self._resolve(path).exists()

        def delete(self, path: str) -> None:
            p = self._resolve(path)
            if p.is_file():
                p.unlink()

    vfs = MappedVfs(str(data_dir))
    agent = CodingAgent(vfs)

    print("=" * 60)
    print("CODING AGENT DEMO")
    print("=" * 60)

    # 1. Show current context
    print("\n1. Current Context:")
    print("-" * 40)
    context = agent.get_context()
    if context["state"]:
        print(f"   Task ID: {context['state'].get('task_id', 'N/A')}")
        print(f"   Status: {context['state'].get('status', 'N/A')}")
        print(f"   Next Step: {context['state'].get('next_step', 'N/A')}")
    else:
        print("   No active task")

    # 2. List available skills
    print("\n2. Available Skills:")
    print("-" * 40)
    skills = agent.list_skills()
    for skill in skills:
        print(f"   - {skill}")

    # 3. Load a skill
    print("\n3. Loading 'code_review' skill:")
    print("-" * 40)
    skill = agent.load_skill("code_review")
    if skill:
        # Show first 200 chars
        print(f"   {skill[:200]}...")

    # 4. Load tool definitions
    print("\n4. Git Tool Commands:")
    print("-" * 40)
    git_tool = agent.load_tool("git")
    if git_tool:
        for cmd in git_tool.get("commands", [])[:5]:
            print(f"   - {cmd['name']}: {cmd['description']}")

    # 5. Show recent conversations
    print("\n5. Recent Conversations (2024-01-15):")
    print("-" * 40)
    convos = agent.get_conversations("2024-01-15")
    for conv in convos[:3]:
        role = conv.get("role", "unknown")
        content = conv.get("content", "")[:50]
        print(f"   [{role}]: {content}...")

    # 6. Load user preferences pattern
    print("\n6. User Preferences:")
    print("-" * 40)
    prefs = agent.load_pattern("user_preferences")
    if prefs:
        # Show first few lines
        lines = prefs.split("\n")[:8]
        for line in lines:
            print(f"   {line}")

    # 7. List code snippets
    print("\n7. Python Code Snippets:")
    print("-" * 40)
    snippets = agent.list_snippets("python")
    for snippet in snippets:
        print(f"   - {snippet}")

    # 8. Load a code snippet
    print("\n8. JWT Auth Snippet (first 300 chars):")
    print("-" * 40)
    jwt_code = agent.load_snippet("python", "jwt_auth.py")
    if jwt_code:
        print(f"   {jwt_code[:300]}...")

    # 9. Use scratch space
    print("\n9. Scratch Space Demo:")
    print("-" * 40)
    agent.save_draft("experiment.py", "# Quick experiment\nprint('hello')")
    print("   Saved draft: experiment.py")
    draft = agent.load_draft("experiment.py")
    print(f"   Content: {draft}")

    print("\n" + "=" * 60)
    print("Demo complete! See README.md for more details.")
    print("=" * 60)


if __name__ == "__main__":
    demo()
