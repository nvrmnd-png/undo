_undo_wrap() {
  local cmd=$1
  shift
  if [ -n "$UNDO_ACTIVE" ] || ! command -v undo >/dev/null 2>&1; then
    command "$cmd" "$@"
    return $?
  fi
  UNDO_ACTIVE=1 command undo exec -- "$cmd" "$@"
  local rc=$?
  if [ "$rc" -eq 125 ]; then
    [ -n "$UNDO_QUIET_FALLBACK" ] || echo "undo: unsupported invocation, running real $cmd (not journaled)" >&2
    command "$cmd" "$@"
    return $?
  fi
  return $rc
}

for _undo_cmd in mv cp rm mkdir rmdir chmod chown ln rename; do
  eval "${_undo_cmd}() { _undo_wrap ${_undo_cmd} \"\$@\"; }"
done
unset _undo_cmd
