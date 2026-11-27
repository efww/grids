import os
import sys
from pathlib import Path

if sys.stdout is None or os.environ.get("PID_IO_NOCONSOLE"):     # pythonw (no console): keep a log next to the projects
    os.environ["PID_IO_NOCONSOLE"] = "1"
    _log = Path(os.environ.get("PID_IO_HOME", Path.cwd() / "projects"))
    _log.mkdir(parents=True, exist_ok=True)
    sys.stdout = sys.stderr = open(_log / "_app.log", "a", encoding="utf-8", buffering=1)

from .server import main  # noqa: E402

main(port=int(os.environ.get("PID_IO_PORT", "8765")), open_browser="--no-browser" not in sys.argv)
