# [GPT-6 Astra] Expected supported owner for captured classifier incident.
import runpy
from pathlib import Path
state=runpy.run_path(str(Path(__file__).with_name('reproduce.py')))
row=next(r for r in state['rows'] if r['number']==6468)
assert row['planned_areas']==['area:ci'], row
