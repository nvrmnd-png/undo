#!/usr/bin/env bash

set -euo pipefail

REPO="${UNDO_REPO:-nvrmnd-png/undo}"
PREFIX="${UNDO_PREFIX:-$HOME/.local}"
BIN_NAME="undo"
INSTALL_DIR="$PREFIX/bin"
MAN_DIR="$PREFIX/share/man/man1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASSUME_YES="${UNDO_YES:-0}"

if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GRN=$'\033[32m'
  YLW=$'\033[33m'; CYN=$'\033[36m'; RST=$'\033[0m'
else
  BOLD=; DIM=; RED=; GRN=; YLW=; CYN=; RST=
fi
msg()  { printf '%s\n' "${CYN}::${RST} $*"; }
ok()   { printf '%s\n' "${GRN}✓${RST} $*"; }
warn() { printf '%s\n' "${YLW}!${RST} $*" >&2; }
die()  { printf '%b\n' "${RED}✗${RST} $*" >&2; exit 1; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

confirm() {
  [ "$ASSUME_YES" = 1 ] && return 0
  [ -r /dev/tty ] || return 1
  local ans
  read -rp "$1 [y/N] " ans </dev/tty || return 1
  [[ "$ans" =~ ^[Yy] ]]
}

install_binary() {
  local src="$1"
  mkdir -p "$INSTALL_DIR"
  install -m755 "$src" "$INSTALL_DIR/$BIN_NAME"
  ok "Installed binary  → $INSTALL_DIR/$BIN_NAME"
}

install_manpage() {
  local src="$1"
  mkdir -p "$MAN_DIR"
  install -m644 "$src" "$MAN_DIR/$BIN_NAME.1"
  ok "Installed man page → $MAN_DIR/$BIN_NAME.1"
}

detect_target() {
  local arch os
  case "$(uname -m)" in
    x86_64|amd64)  arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) die "unsupported architecture: $(uname -m) — try: $0 install --source" ;;
  esac
  case "$(uname -s)" in
    Linux)  os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *) die "unsupported OS: $(uname -s) — try: $0 install --source" ;;
  esac
  printf '%s-%s\n' "$arch" "$os"
}

download() {
  local url="$1" out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
  else
    die "need curl or wget to download prebuilt binaries"
  fi
}

verify_sha() {
  local file="$1" sumfile="$2" expected actual
  need_cmd sha256sum
  expected="$(awk 'NR==1{print $1}' "$sumfile")"
  actual="$(sha256sum "$file" | awk '{print $1}')"
  [ -n "$expected" ] || die "checksum file is empty"
  [ "$expected" = "$actual" ] \
    || die "checksum mismatch!\n  expected $expected\n  actual   $actual"
}

install_from_source() {
  need_cmd cargo
  [ -f "$SCRIPT_DIR/Cargo.toml" ] \
    || die "Cargo.toml not found next to manager.sh — run from the repo, or use --prebuilt"
  msg "Building release binary (first build can take a minute) ..."
  ( cd "$SCRIPT_DIR" && cargo build --release --locked )
  install_binary "$SCRIPT_DIR/target/release/$BIN_NAME"
  [ -f "$SCRIPT_DIR/doc/$BIN_NAME.1" ] && install_manpage "$SCRIPT_DIR/doc/$BIN_NAME.1"
}

install_prebuilt() {
  need_cmd tar
  local target url_base tarball tmp bin man
  target="$(detect_target)"
  url_base="https://github.com/$REPO/releases/latest/download"
  tarball="${BIN_NAME}-${target}.tar.gz"
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

  msg "Target: $target"
  msg "Downloading $tarball ..."
  download "$url_base/$tarball" "$tmp/$tarball" \
    || die "download failed — no prebuilt for $target (or wrong UNDO_REPO).\n  Fall back to: $0 install --source"

  if download "$url_base/$tarball.sha256" "$tmp/$tarball.sha256" 2>/dev/null; then
    msg "Verifying checksum ..."
    verify_sha "$tmp/$tarball" "$tmp/$tarball.sha256"
    ok "Checksum verified"
  else
    warn "No checksum published for this release."
    confirm "Continue without checksum verification?" || die "aborted"
  fi

  msg "Extracting ..."
  tar -xzf "$tmp/$tarball" -C "$tmp"
  bin="$(find "$tmp" -type f -name "$BIN_NAME" | head -n1)"
  [ -n "$bin" ] || die "binary '$BIN_NAME' not found inside the archive"
  install_binary "$bin"
  man="$(find "$tmp" -type f -name "$BIN_NAME.1" | head -n1)"
  [ -n "$man" ] && install_manpage "$man"
}

ensure_path() {
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) return 0 ;;
  esac
  warn "$INSTALL_DIR is not in your PATH."
  local rc="" line="export PATH=\"$INSTALL_DIR:\$PATH\""
  case "${SHELL##*/}" in
    zsh)  rc="$HOME/.zshrc" ;;
    bash) rc="$HOME/.bashrc" ;;
  esac
  if [ -n "$rc" ] && confirm "Add $INSTALL_DIR to your PATH in $rc?"; then
    printf '\n# added by undo manager.sh\n%s\n' "$line" >> "$rc"
    ok "Updated $rc — run 'source $rc' or open a new terminal."
  else
    warn "Add this to your shell startup file so '$BIN_NAME' is found:"
    printf '    %s\n' "$line" >&2
  fi
}

shell_hint() {
  local shell rc eval_line
  shell="${SHELL##*/}"
  case "$shell" in
    zsh)  rc="$HOME/.zshrc";        eval_line='eval "$(undo init zsh)"' ;;
    bash) rc="$HOME/.bashrc";       eval_line='eval "$(undo init bash)"' ;;
    fish) rc="$HOME/.config/fish/config.fish"; eval_line='undo init fish | source' ;;
    *)    rc="your shell rc";       eval_line='eval "$(undo init zsh)"' ;;
  esac
  printf '\n%sOne more step — enable the shell integration%s\n' "$BOLD" "$RST"
  printf 'undo only records commands that go through its wrappers. Add this to %s%s%s:\n' \
    "$BOLD" "$rc" "$RST"
  printf '    %s%s%s\n' "$CYN" "$eval_line" "$RST"
  printf '%sThen open a new shell and your mv/cp/rm/… become undoable.%s\n' "$DIM" "$RST"
}

verify_install() {
  if [ -x "$INSTALL_DIR/$BIN_NAME" ]; then
    ok "$("$INSTALL_DIR/$BIN_NAME" --version 2>/dev/null || echo "$BIN_NAME installed")"
    msg "Docs: ${BOLD}man $BIN_NAME${RST}"
    shell_hint
  fi
}

choose_method() {
  if [ "$ASSUME_YES" = 1 ]; then echo source; return; fi
  [ -r /dev/tty ] || die "no terminal for interactive choice — pass --source or --prebuilt"
  {
    printf '\nHow do you want to install %sundo%s?\n' "$BOLD" "$RST"
    printf '  %s1%s) Build from source  %s(needs Rust; always matches your machine)%s\n' "$BOLD" "$RST" "$DIM" "$RST"
    printf '  %s2%s) Prebuilt binary    %s(fast; downloads from GitHub releases)%s\n' "$BOLD" "$RST" "$DIM" "$RST"
  } >/dev/tty
  local c=""
  read -rp "Choice [1/2]: " c </dev/tty || true
  case "$c" in
    1) echo source ;;
    2) echo prebuilt ;;
    *) die "invalid choice: '$c'" ;;
  esac
}

do_install() {
  local method="${1:-}"
  [ -n "$method" ] || method="$(choose_method)"
  case "$method" in
    source|--source)     install_from_source ;;
    prebuilt|--prebuilt) install_prebuilt ;;
    *) die "unknown install method: $method" ;;
  esac
  ensure_path
  verify_install
}

do_uninstall() {
  local removed=0
  [ -e "$INSTALL_DIR/$BIN_NAME" ]  && { rm -f "$INSTALL_DIR/$BIN_NAME";  ok "Removed $INSTALL_DIR/$BIN_NAME";  removed=1; }
  [ -e "$MAN_DIR/$BIN_NAME.1" ]    && { rm -f "$MAN_DIR/$BIN_NAME.1";    ok "Removed $MAN_DIR/$BIN_NAME.1";    removed=1; }
  [ "$removed" = 1 ] || warn "nothing to uninstall under $PREFIX"
  warn "Remove the 'undo init' line from your shell rc to disable the wrappers."
  warn "Your journal and trash are left untouched (in \$XDG_DATA_HOME)."
}

usage() {
  cat <<EOF
${BOLD}undo manager${RST}

Usage:
  ./manager.sh [install]            interactive install (source or prebuilt)
  ./manager.sh install --source     build from source (needs cargo)
  ./manager.sh install --prebuilt   download a prebuilt binary
  ./manager.sh uninstall            remove binary + man page
  ./manager.sh --help

Env: UNDO_REPO (default $REPO), UNDO_PREFIX (default $PREFIX),
     UNDO_YES=1 to skip prompts.
EOF
}

main() {
  local cmd="${1:-install}"
  case "$cmd" in
    install)             shift || true; do_install "${1:-}" ;;
    --source|--prebuilt) do_install "$cmd" ;;
    uninstall|remove)    do_uninstall ;;
    -h|--help|help)      usage ;;
    *) die "unknown command: $cmd  (try --help)" ;;
  esac
}

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  main "$@"
fi
