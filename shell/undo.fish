function _undo_wrap
    set -l cmd $argv[1]
    set -e argv[1]
    if set -q UNDO_ACTIVE; or not command -q undo
        command $cmd $argv
        return $status
    end
    env UNDO_ACTIVE=1 undo exec -- $cmd $argv
    set -l rc $status
    if test $rc -eq 125
        if not set -q UNDO_QUIET_FALLBACK
            echo "undo: unsupported invocation, running real $cmd (not journaled)" >&2
        end
        command $cmd $argv
        return $status
    end
    return $rc
end

for _undo_cmd in mv cp rm mkdir rmdir chmod chown ln rename
    eval "function $_undo_cmd --wraps $_undo_cmd; _undo_wrap $_undo_cmd \$argv; end"
end
set -e _undo_cmd
