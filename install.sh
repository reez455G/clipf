#!/bin/sh
# clipf installer. POSIX sh, no bashisms, no dependencies beyond curl-or-wget.
#
#   curl -fsSL https://raw.githubusercontent.com/reez455G/clipf/main/install.sh | sh
#   ./install.sh --remote user@host
#
set -eu

REPO="reez455G/clipf"

VERSION="${CLIPF_VERSION:-latest}"
BIN_DIR="${CLIPF_BIN_DIR:-}"
BIN_DIR_EXPLICIT=0
REMOTE=""
NO_PATH=0
FROM_SOURCE=0

die() {
	echo "install.sh: $*" >&2
	exit 1
}

say() {
	echo "install.sh: $*"
}

usage() {
	cat <<'EOF'
Usage: install.sh [--version VERSION] [--bin-dir DIR] [--remote [USER@]HOST] [--no-path] [--build]

  --version VERSION   release tag to install, e.g. v0.4.0 (default: latest)
  --bin-dir DIR       install target (default: /usr/local/bin when root,
                      $PREFIX/bin under Termux, otherwise $HOME/.local/bin)
  --remote HOST       install onto HOST over ssh instead of this machine
  --no-path           do not touch any shell rc file
  --build             skip the download and build from source with cargo

Environment: CLIPF_VERSION, CLIPF_BIN_DIR are read as defaults before flags.
EOF
}

while [ $# -gt 0 ]; do
	case "$1" in
	--version)
		[ $# -ge 2 ] || die "--version needs an argument"
		VERSION="$2"
		shift 2
		;;
	--version=*)
		VERSION="${1#--version=}"
		shift
		;;
	--bin-dir)
		[ $# -ge 2 ] || die "--bin-dir needs an argument"
		BIN_DIR="$2"
		BIN_DIR_EXPLICIT=1
		shift 2
		;;
	--bin-dir=*)
		BIN_DIR="${1#--bin-dir=}"
		BIN_DIR_EXPLICIT=1
		shift
		;;
	--remote)
		[ $# -ge 2 ] || die "--remote needs an argument"
		REMOTE="$2"
		shift 2
		;;
	--remote=*)
		REMOTE="${1#--remote=}"
		shift
		;;
	--no-path)
		NO_PATH=1
		shift
		;;
	--build)
		FROM_SOURCE=1
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		usage >&2
		die "unknown option: $1"
		;;
	esac
done

# Map `uname -s` / `uname -m` to a Rust target triple. Takes both values as
# arguments so the same function serves the local and the remote host.
# Linux always gets the musl build: statically linked, so one asset covers
# glibc 2.17 (CentOS 7) through current distros, Alpine, and Android/Termux.
detect_target() {
	case "$1" in
	Linux)
		case "$2" in
		x86_64 | amd64) echo "x86_64-unknown-linux-musl" ;;
		aarch64 | arm64) echo "aarch64-unknown-linux-musl" ;;
		*) echo "" ;;
		esac
		;;
	Darwin)
		case "$2" in
		x86_64) echo "x86_64-apple-darwin" ;;
		arm64) echo "aarch64-apple-darwin" ;;
		*) echo "" ;;
		esac
		;;
	MINGW* | MSYS* | CYGWIN*)
		case "$2" in
		x86_64) echo "x86_64-pc-windows-msvc" ;;
		*) echo "" ;;
		esac
		;;
	*) echo "" ;;
	esac
}

asset_for() {
	case "$1" in
	*-pc-windows-msvc) echo "clipf-$1.zip" ;;
	*) echo "clipf-$1.tar.gz" ;;
	esac
}

base_url() {
	if [ "$VERSION" = latest ]; then
		echo "https://github.com/$REPO/releases/latest/download"
	else
		echo "https://github.com/$REPO/releases/download/$VERSION"
	fi
}

fetch() {
	# fetch URL DEST
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -qO "$2" "$1"
	else
		die "need curl or wget"
	fi
}

sha256_of() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d' ' -f1
	else
		echo ""
	fi
}

verify_checksum() {
	# verify_checksum DIR ASSET
	want=$(awk -v a="$2" '$2 == a || $2 == "*" a { print $1; exit }' "$1/SHA256SUMS")
	[ -n "$want" ] || die "no checksum for $2 in SHA256SUMS"
	got=$(sha256_of "$1/$2")
	if [ -z "$got" ]; then
		say "warning: no sha256 tool, skipping checksum verification"
		return 0
	fi
	[ "$got" = "$want" ] || die "checksum mismatch for $2"
	say "checksum verified: $2"
}

# Download, verify and unpack the asset for TARGET into DIR, leaving the binary
# at DIR/clipf (or DIR/clipf.exe).
#
# POSIX sh has no locals, so every variable here is prefixed to keep it clear
# of the caller's own (an unprefixed `dir` silently overwrote the remote
# install directory once already).
stage_binary() {
	# stage_binary DIR TARGET
	sb_dir="$1"
	sb_asset=$(asset_for "$2")
	sb_url="$(base_url)/$sb_asset"

	say "downloading $sb_asset"
	fetch "$sb_url" "$sb_dir/$sb_asset" || die "cannot download $sb_url"
	fetch "$(base_url)/SHA256SUMS" "$sb_dir/SHA256SUMS" || die "cannot download SHA256SUMS"
	verify_checksum "$sb_dir" "$sb_asset"

	case "$sb_asset" in
	*.tar.gz)
		command -v tar >/dev/null 2>&1 || die "need tar to unpack $sb_asset"
		tar -xzf "$sb_dir/$sb_asset" -C "$sb_dir" || die "cannot unpack $sb_asset"
		;;
	*.zip)
		command -v unzip >/dev/null 2>&1 || die "need unzip to unpack $sb_asset"
		unzip -q -o "$sb_dir/$sb_asset" -d "$sb_dir" || die "cannot unpack $sb_asset"
		;;
	esac
}

# Termux has no /usr/local and its own $PREFIX tree; everything else gets
# /usr/local/bin as root or ~/.local/bin as a user.
is_termux() {
	[ -n "${TERMUX_VERSION:-}" ] && return 0
	case "${PREFIX:-}" in
	*com.termux*) return 0 ;;
	esac
	return 1
}

default_bin_dir() {
	if is_termux && [ -n "${PREFIX:-}" ]; then
		echo "$PREFIX/bin"
	elif [ "$(id -u 2>/dev/null || echo 1000)" = 0 ]; then
		echo "/usr/local/bin"
	else
		echo "$HOME/.local/bin"
	fi
}

install_file() {
	# install_file SRC DEST_DIR
	mkdir -p "$2"
	if command -v install >/dev/null 2>&1; then
		install -m 0755 "$1" "$2/clipf"
	else
		cp "$1" "$2/clipf"
		chmod 0755 "$2/clipf"
	fi
}

rc_file_for_shell() {
	case "$(basename "${SHELL:-/bin/sh}")" in
	bash) echo "$HOME/.bashrc" ;;
	zsh) echo "$HOME/.zshrc" ;;
	fish) echo "$HOME/.config/fish/conf.d/clipf.fish" ;;
	*) echo "$HOME/.profile" ;;
	esac
}

ensure_on_path() {
	# ensure_on_path BIN_DIR
	case ":$PATH:" in
	*":$1:"*) return 0 ;;
	esac
	[ "$NO_PATH" -eq 0 ] || {
		say "$1 is not on PATH. Add it yourself:  export PATH=\"$1:\$PATH\""
		return 0
	}

	rc=$(rc_file_for_shell)
	mkdir -p "$(dirname "$rc")"
	if [ -f "$rc" ] && grep -q '^# >>> clipf >>>$' "$rc" 2>/dev/null; then
		say "PATH block already present in $rc"
		return 0
	fi

	case "$rc" in
	*.fish)
		{
			echo "# >>> clipf >>>"
			echo "fish_add_path $1"
			echo "# <<< clipf <<<"
		} >>"$rc"
		;;
	*)
		{
			echo "# >>> clipf >>>"
			echo "case \":\$PATH:\" in *\":$1:\"*) ;; *) PATH=\"$1:\$PATH\" ;; esac"
			echo "# <<< clipf <<<"
		} >>"$rc"
		;;
	esac
	say "added $1 to PATH in $rc — run: exec \$SHELL"
}

build_from_source() {
	# build_from_source BIN_DIR OS ARCH
	if ! command -v cargo >/dev/null 2>&1; then
		if [ "$FROM_SOURCE" -eq 1 ]; then
			die "--build needs cargo. Install Rust from https://rustup.rs then re-run"
		fi
		die "no prebuilt binary for $2/$3 and cargo is not installed. Install Rust from https://rustup.rs then re-run with --build"
	fi
	root="${1%/bin}"
	say "building from source with cargo into $root"
	if [ "$VERSION" = latest ]; then
		cargo install --git "https://github.com/$REPO" --root "$root" --locked clipf
	else
		cargo install --git "https://github.com/$REPO" --tag "$VERSION" --root "$root" --locked clipf
	fi
	# cargo always writes to <root>/bin, which is $1 itself only when $1 ends
	# in /bin. Anywhere else, move the binary where the caller asked for it.
	if [ "$root/bin" != "$1" ]; then
		install_file "$root/bin/clipf" "$1"
		rm -f "$root/bin/clipf"
	fi
}

install_local() {
	os=$(uname -s)
	arch=$(uname -m)
	[ -n "$BIN_DIR" ] || BIN_DIR=$(default_bin_dir)

	target=$(detect_target "$os" "$arch")
	if [ "$FROM_SOURCE" -eq 1 ] || [ -z "$target" ]; then
		build_from_source "$BIN_DIR" "$os" "$arch"
	else
		TMP=$(mktemp -d)
		trap 'rm -rf "$TMP"' EXIT INT TERM
		stage_binary "$TMP" "$target"
		bin="$TMP/clipf"
		[ -f "$bin" ] || bin="$TMP/clipf.exe"
		[ -f "$bin" ] || die "archive did not contain a clipf binary"
		install_file "$bin" "$BIN_DIR"
	fi

	ensure_on_path "$BIN_DIR"
	"$BIN_DIR/clipf" --version
	say "installed. Run 'clipf --check' to verify this environment."
}

# Every ssh here passes -n: this script is often itself read from stdin
# (`curl … | sh -s -- --remote HOST`), and an ssh that inherits stdin would
# swallow the rest of the script.
install_remote() {
	say "probing $REMOTE over ssh"
	uname_out=$(ssh -n "$REMOTE" 'uname -s; uname -m') || die "cannot reach $REMOTE over ssh"
	os=$(echo "$uname_out" | sed -n 1p)
	arch=$(echo "$uname_out" | sed -n 2p)

	target=$(detect_target "$os" "$arch")
	[ -n "$target" ] || die "no prebuilt binary for $os/$arch on $REMOTE"

	if [ "$BIN_DIR_EXPLICIT" -eq 1 ]; then
		dir="$BIN_DIR"
	else
		remote_home=$(ssh -n "$REMOTE" 'echo $HOME') || die "cannot reach $REMOTE over ssh"
		dir="$remote_home/.local/bin"
	fi

	# Download and verify locally, so the remote needs neither internet access
	# nor curl/tar.
	TMP=$(mktemp -d)
	trap 'rm -rf "$TMP"' EXIT INT TERM
	stage_binary "$TMP" "$target"
	bin="$TMP/clipf"
	[ -f "$bin" ] || bin="$TMP/clipf.exe"
	[ -f "$bin" ] || die "archive did not contain a clipf binary"

	# $dir is resolved on this side (from the remote $HOME or --bin-dir), so the
	# client-side expansion SC2029 warns about is exactly what is wanted here.
	# shellcheck disable=SC2029
	ssh -n "$REMOTE" "mkdir -p '$dir'" || die "cannot create $dir on $REMOTE"
	scp -q "$bin" "$REMOTE:$dir/clipf" || die "cannot copy clipf to $REMOTE"
	# shellcheck disable=SC2029
	ssh -n "$REMOTE" "chmod 0755 '$dir/clipf' && '$dir/clipf' --version" ||
		die "clipf does not run on $REMOTE"

	say "deployed to $REMOTE:$dir/clipf"
	say "if 'clipf' is not found on $REMOTE, add $dir to its PATH."
}

if [ -n "$REMOTE" ]; then
	install_remote
else
	install_local
fi
