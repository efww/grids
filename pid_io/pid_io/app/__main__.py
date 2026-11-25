import os
import sys

from .server import main

main(port=int(os.environ.get("PID_IO_PORT", "8765")), open_browser="--no-browser" not in sys.argv)
