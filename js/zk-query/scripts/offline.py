"""Run checks offline from a source checkout or a built package."""
from pathlib import Path
import runpy
package = Path(__file__).resolve().parent.parent
source = package.parent / 'zk-shared' / 'offline.py'
bundled = package / 'internal' / 'offline.py'
runpy.run_path(str(bundled if bundled.is_file() else source), run_name='__main__')
