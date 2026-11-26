"""Where AI answers come from.

  none        : AI off
  manual      : copy the prompt into any chat AI (e.g. M365 Copilot), paste the answer back
  copilot_cli : run the GitHub Copilot CLI on this PC - one click

The CLI command is a template kept in ai_settings.json so it can be adapted
to the installed Copilot CLI version without touching code, e.g.
  ["copilot", "-p", "{prompt}"]                 prompt as an argument
  ["copilot", "-p", "@{prompt_file}"]           prompt in a file
  ["copilot"]  with "stdin": true               prompt on standard input
'{model}' is replaced by the model name when one is set.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

DEFAULTS = {
    "provider": "none",
    "command": ["copilot", "-p", "{prompt}"],
    "stdin": False,
    "model": "",
    "timeout": 300,
}


class NeedsManual(Exception):
    """Raised by the manual provider: the UI shows the prompt and waits for a pasted answer."""

    def __init__(self, prompt: str):
        super().__init__("manual answer needed")
        self.prompt = prompt


def load_settings(home: Path) -> dict:
    f = Path(home) / "ai_settings.json"
    s = dict(DEFAULTS)
    if f.exists():
        s.update(json.loads(f.read_text(encoding="utf-8")))
    s["_home"] = str(home)
    return s


def save_settings(home: Path, s: dict):
    keep = {k: s[k] for k in DEFAULTS if k in s}
    (Path(home) / "ai_settings.json").write_text(json.dumps(keep, ensure_ascii=False, indent=1), encoding="utf-8")


def ask(settings: dict, prompt: str) -> str:
    p = settings.get("provider", "none")
    if p == "none":
        raise RuntimeError("AI 연결이 꺼져 있습니다 (AI 설정에서 선택)")
    if p == "manual":
        raise NeedsManual(prompt)
    if p == "copilot_cli":
        return _copilot(settings, prompt)
    raise RuntimeError(f"unknown provider {p}")


def _copilot(settings: dict, prompt: str) -> str:
    cmd = list(settings.get("command") or DEFAULTS["command"])
    exe = shutil.which(cmd[0]) or shutil.which(cmd[0] + ".cmd") or shutil.which(cmd[0] + ".exe")
    if not exe:
        raise RuntimeError(f"'{cmd[0]}' 명령을 찾을 수 없습니다. GitHub Copilot CLI 설치와 PATH를 확인하세요.")
    cmd[0] = exe
    if exe.lower().endswith((".cmd", ".bat")) and not settings.get("stdin"):
        # Windows runs .cmd through cmd.exe, which cuts an argument at the first line break -
        # the CLI then got only the first line of the question. Send it as one line.
        prompt = " ".join(prompt.split())
    tmp = None
    if any("{prompt_file}" in c for c in cmd):
        tmp = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8")
        tmp.write(prompt)
        tmp.close()
    model = settings.get("model") or ""
    args = []
    for c in cmd:
        if c == "{model}" and not model:
            if args and args[-1].startswith("--model"):
                args.pop()
            continue
        args.append(c.replace("{prompt}", prompt).replace("{prompt_file}", tmp.name if tmp else "")
                     .replace("{model}", model))
    try:
        r = subprocess.run(args, input=prompt if settings.get("stdin") else None, capture_output=True,
                           text=True, encoding="utf-8", errors="replace",
                           timeout=int(settings.get("timeout") or 300))
    finally:
        if tmp:
            os.unlink(tmp.name)
    try:                                   # keep the last raw answer for troubleshooting
        (Path(settings.get("_home") or ".") / "ai_last_answer.txt").write_text(
            f"$ {' '.join(args[:2])} ...\n--- stdout ---\n{r.stdout}\n--- stderr ---\n{r.stderr}", encoding="utf-8")
    except OSError:
        pass
    if r.returncode != 0 and not r.stdout.strip():
        raise RuntimeError(f"Copilot CLI 오류 (code {r.returncode}): {r.stderr.strip()[:400]}")
    return r.stdout


def test(settings: dict) -> str:
    """Round trip with a tiny prompt; returns the raw answer."""
    return ask(settings, 'Reply with exactly this JSON and nothing else: {"ok": true}')
