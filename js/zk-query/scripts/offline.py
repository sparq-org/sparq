"""Run the package check through its bundled offline wrapper."""
from pathlib import Path
import runpy
runpy.run_path(str(Path(__file__).resolve().parent.parent / 'internal' / 'offline.py'), run_name='__main__')
