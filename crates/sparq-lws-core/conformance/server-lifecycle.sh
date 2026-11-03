#!/usr/bin/env bash
# Shared by the two conformance drivers. The caller supplies server_env and server_ready.
# Keep the exact child PID: never find or signal another run's server by process name.

SERVER_PID=""
DATA_DIR=""
SERVER_BIN=${SERVER_BIN:?caller must set SERVER_BIN}

make_data_dir() {
  DATA_DIR=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/sparq-conformance.XXXXXX")
}

start_server() {
  local log=$1
  # Launch the server DETACHED into its own session (load-bearing). The CTH runs as
  # `docker run -i --network host …`, and on Docker Desktop that `-i` foreground attach can
  # forward a SIGTERM to this script's process group when the container exits, which would
  # kill a same-group server mid-run. A python os.setsid() wrapper works on Linux and macOS
  # (which has no `setsid`). exec preserves $! across the shell function and the wrapper.
  (server_env exec python3 -c 'import os,sys; os.setsid(); os.execv(sys.argv[1], sys.argv[1:])' "$SERVER_BIN") \
    > "$log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 100); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      echo "ERROR: server exited during startup; see $log" >&2
      return 1
    fi
    if server_ready; then return 0; fi
    sleep 0.2
  done
  echo "ERROR: server did not become ready; see $log" >&2
  return 1
}

stop_server() {
  local signal=$1 status=0 polls=150
  # Graceful exit includes index compaction; forced exit has no storage work to drain.
  if [ "$signal" = TERM ]; then polls=1500; fi
  kill "-$signal" "$SERVER_PID"
  for _ in $(seq 1 "$polls"); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
    sleep 0.2
  done
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "ERROR: server did not exit after SIG$signal" >&2
    return 1
  fi
  wait "$SERVER_PID" || status=$?
  SERVER_PID=""
  if [ "$signal" = TERM ]; then
    [ "$status" = 0 ] || { echo "ERROR: graceful shutdown exited $status" >&2; return 1; }
  else
    [ "$status" = 137 ] || { echo "ERROR: SIGKILL exited $status (expected 137)" >&2; return 1; }
  fi
}

cleanup_server() {
  if [ -n "$SERVER_PID" ]; then
    stop_server TERM || {
      kill -KILL "$SERVER_PID" 2>/dev/null || true
      wait "$SERVER_PID" 2>/dev/null || true
    }
  fi
  if [ -n "$DATA_DIR" ]; then rm -rf -- "$DATA_DIR"; fi
}
