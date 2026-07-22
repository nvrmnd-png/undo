# undo

> `mv file.txt archive/`, oops, `undo`. Reversible `mv`, `cp`, `rm` and friends, with a freedesktop trash safety net.

![License](https://img.shields.io/badge/license-MIT-blue) ![Rust](https://img.shields.io/badge/rust-2024-orange) ![Shells](https://img.shields.io/badge/shell-zsh%20%7C%20bash%20%7C%20fish-lightgrey)

Shell filesystem commands are unforgiving: once you `mv` over a file or `rm` a
directory, there is no built-in way back. **undo** journals every supported
operation and reverses it on demand, and it never deletes anything outright,
so even `rm` is recoverable.

![undo demo: mv then undo restores the file, rm -r then undo restores it from the trash](.github/demo.gif)

It works by *intercepting* commands rather than watching them: a small shell
integration routes `mv`, `cp`, `rm`, … through undo, which performs the
operation itself in Rust and records exactly how to invert it. Deletions go to
the freedesktop.org trash, overwritten files are backed up there first, and
every rollback verifies file integrity before touching anything.

## Supported commands

`mv` · `cp` · `rm` · `mkdir` · `rmdir` · `chmod` · `chown` · `ln` · `rename`

`rm` and `rmdir` move to the trash instead of deleting. Overwrites (`mv`, `cp`,
`ln -f`) park the displaced file in the trash so they are fully reversible.
`rename` uses the Perl `s/old/new/` syntax.

## Install

```sh
git clone https://github.com/nvrmnd-png/undo
cd undo
./manager.sh                 # interactive: build from source or fetch a prebuilt binary
```

`manager.sh install --source` builds with `cargo` and installs to
`~/.local/bin`; `--prebuilt` downloads a release binary and verifies its
SHA-256. Override the location with `UNDO_PREFIX`. Or use cargo directly:

```sh
cargo install --path .
```

### Enable the shell integration

undo only records commands that go through its wrappers. Add one line to your
shell startup file:

```sh
# ~/.zshrc
eval "$(undo init zsh)"

# ~/.bashrc
eval "$(undo init bash)"

# ~/.config/fish/config.fish
undo init fish | source
```

Open a new shell and your everyday `mv`/`cp`/`rm`/… become undoable. Bypass the
wrapper for a single call with `command mv …` or `\mv …`.

## Usage

| Command | Does |
|---|---|
| `undo` | undo the most recent operation |
| `undo redo` | re-apply the most recently undone operation |
| `undo list` | show the undo and redo stacks |
| `undo history` | show the full journal (`-n N`, `--all`) |
| `undo search <name>` | find entries by filename or path |
| `undo log` | activity log: when, which command, which files |
| `undo show <id>` | show one entry in detail |
| `undo clear` | forget the journal (never touches the trash) |
| `undo prune` | drop old journal entries (`--older-than N`, `--empty-trash`) |
| `undo repair` | check and rebuild a damaged journal database |
| `undo config` | edit settings in a TUI (`show`, `reset`) |
| `undo update` | update to the latest release (`--check`) |
| `undo tui` | browse and undo/redo interactively |
| `undo init <shell>` | print the shell integration snippet |

Global flags: `--force` (override an integrity refusal; displaced files go to
the trash), `--dry-run` (report what would happen), and `--json` / `--yaml`
(stable machine output, `schema: 1`).

```console
$ undo list
Undo stack (newest first):
#8  2026-07-14 09:14  applied      rm -r build/
#7  2026-07-14 09:12  applied      mv report.txt archive/

$ undo history --json | jq '.entries[0].command'
"rm"
```

### Interactive TUI

`undo tui` opens a colored browser: a list of journal entries with a detail
pane and a live integrity badge.

| Key | Action |
|---|---|
| `j` / `k` | move down / up |
| `Tab` | cycle filter |
| `v` | verify selected entry |
| `u` | undo selected entry |
| `r` | redo selected entry |
| `f` | toggle `--force` |
| `?` | help |
| `q` | quit |

## Configuration

Settings live in `~/.config/undo/config.toml`. Edit them in a TUI with
`undo config`, print them with `undo config show`, or write the file yourself:

```toml
[cleanup]
enabled = true          # run a light prune automatically (journal only)
max_age_days = 90
max_database_size = 500

[storage]
path = "~/.undo"        # where the journal lives (the trash stays in XDG)

[exclude]
paths = ["node_modules", ".cache"]   # operations here are not journaled

[logging]
enabled = true          # append each journaled operation to undo.log

[plugins]
mycp = "cp"             # your own command names, mapped to a built-in
trashit = "rm"
```

### Plugins (command aliases)

If you have your own wrapper commands, map them to a built-in under
`[plugins]` (each value must be one of the nine supported commands). They then
run through undo just like the originals and become undoable. After adding an
alias, re-run `eval "$(undo init zsh)"` (or open a new shell) so the wrapper
picks it up.

### Staying current

`undo update` checks GitHub for a newer release and installs it (verifying the
SHA-256); `undo update --check` only reports whether one is available.

`undo prune` drops journal entries older than `--older-than N` days (default
`max_age_days`). By default it keeps trashed files (you can still restore them
from your file manager); pass `--empty-trash` to permanently delete them too.
`--dry-run` shows what would be removed without touching anything.

## Safety

- **Nothing is ever deleted.** `rm`/`rmdir` use the freedesktop trash; every
  destructive inverse (removing a copy, replacing an overwrite) goes through
  the trash too. Cross-device moves are a verified copy before any removal.
- **Integrity is checked before every rollback.** Content is fingerprinted
  with BLAKE3 (size + mtime above 64 MiB); a file modified since the operation
  refuses to roll back unless you pass `--force`.
- **Your data only.** The journal is private (`0600`, ownership-checked) and
  entries from another user are refused.
- **Symlinks are never followed** during inspection, and operations are
  serialized by a lock with a re-check immediately before each mutation.

### Limitations

undo can only reverse what went through its wrappers. `sudo`, scripts, cron
and GUI file managers bypass them (a stale journal is caught by verification,
not blindly replayed). ACLs, xattrs, and hard-link topology inside copied trees
are not preserved. Non-UTF-8 paths fall back to the real command. Clearing the
journal does not empty the trash. See `man undo` for the full list.

## How it works

Every operation is stored in a per-user SQLite journal (`$XDG_DATA_HOME/undo`)
with its command line, working directory, the pre/post state of each affected
node, and the exact inverse. `undo`/`redo` walk this as a stack; a new
operation clears the redo stack, like an editor. A partial failure mid-rollback
compensates back to a consistent state, and because everything displaced lives
in the trash, nothing is lost even in the worst case.

## Development

```sh
cargo test            # 150+ unit and integration tests
cargo clippy --all-targets
cargo fmt --check
```

## Support

If undo saved you from a bad `rm`, consider chipping in:

| Asset | Address |
|---|---|
| BTC | `bc1qmgnz54x4epaeuvz558z4d7n3a3dqk3vldsgcnt` |
| XMR | `43eLK3JjmkDiWAuP9b5N8sahtzQQByaVSEPbZvVTctXAYJ1cHGXbVL3f8PSREnmnSsY4rMkhx14UA8vxc5sfz4ZhDfxKkg5` |
| ETH | `0x8CF98DeF5d716E10697E5905caff34b405dcd4fF` |
| SOL | `DFwWy5EMB5QRd1FTV81ft5WYyH6uHecRvPsXkLNRyh1s` |

## License

MIT © 2026 nvrmnd. See [LICENSE](LICENSE).

(v0.1.0 was released under GPLv3 and stays GPLv3; v0.1.1 onward is MIT.)
