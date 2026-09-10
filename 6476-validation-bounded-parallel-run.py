import subprocess,sys
r=subprocess.run([sys.argv[1]],timeout=30)
sys.exit(r.returncode)
