#!/usr/bin/env bash
# package-dist.sh — build the release binary and assemble swapdock-dist/.
#
#   bash scripts/package-dist.sh
#   bash scripts/package-dist.sh --target x86_64-unknown-linux-musl
#   bash scripts/package-dist.sh --no-build --binary /tmp/swapdock
#
# The resulting tree is the argument install/install-swapdock.sh expects:
#
#   swapdock-dist/
#     swapdock
#     apps/README.md
#     nginx/*.conf
#     systemd/swapdock.service
#     logrotate-swapdock
#     install-swapdock.sh
#     uninstall-swapdock.sh
#     LICENSE-MIT
#
# The script only writes the output directory. It never installs anything and
# never touches the host; the installer does that, under sudo, on purpose.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo_root/swapdock-dist"
target=""
binary=""
do_build=1

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
usage: bash scripts/package-dist.sh [options]

Build a release binary with `cargo build --release --locked` and assemble a
self-contained swapdock-dist/ tree for install/install-swapdock.sh.

options:
  --target <triple>  Build for a Rust target triple (passed to cargo)
  --binary <path>    Use an existing binary instead of building (implies --no-build)
  --no-build         Do not run cargo; requires --binary or an existing build
  --out <dir>        Output directory (default: ./swapdock-dist)
  -h, --help         Show this help

examples:
  bash scripts/package-dist.sh
  bash scripts/package-dist.sh --target x86_64-unknown-linux-musl
  bash scripts/package-dist.sh --no-build \
      --binary target/aarch64-unknown-linux-musl/release/swapdock
EOF
}

while (($#)); do
    case "$1" in
        --target)
            (($# >= 2)) || die "--target needs a value"
            target="$2"
            shift 2
            ;;
        --binary)
            (($# >= 2)) || die "--binary needs a value"
            binary="$2"
            do_build=0
            shift 2
            ;;
        --no-build)
            do_build=0
            shift
            ;;
        --out)
            (($# >= 2)) || die "--out needs a value"
            out="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

case "$out" in
    /*) ;;
    *) out="$PWD/$out" ;;
esac
out="${out%/}"
case "$out" in
    "" | "/" | "." | "..")
        die "refusing to use an unsafe output directory: $out"
        ;;
esac

sources=(
    install/install-swapdock.sh
    install/uninstall-swapdock.sh
    install/logrotate-swapdock
    install/apps/README.md
    install/nginx/00-swapdock-http.conf
    install/nginx/proxy-common.conf
    install/nginx/spa-fallback.conf
)
for src in "${sources[@]}"; do
    [[ -f "$repo_root/$src" ]] || die "missing source file: $src"
done

if ((do_build)); then
    cargo_args=(build --release --locked)
    if [[ -n "$target" ]]; then
        cargo_args+=(--target "$target")
    fi
    printf 'building: cargo %s\n' "${cargo_args[*]}"
    (cd "$repo_root" && cargo "${cargo_args[@]}")
fi

if [[ -z "$binary" ]]; then
    target_root="${CARGO_TARGET_DIR:-target}"
    case "$target_root" in
        /*) ;;
        *) target_root="$repo_root/$target_root" ;;
    esac
    if [[ -n "$target" ]]; then
        binary="$target_root/$target/release/swapdock"
    else
        binary="$target_root/release/swapdock"
    fi
fi

[[ -f "$binary" ]] || die "release binary not found: $binary (build it, or pass --binary)"
[[ -x "$binary" ]] || die "release binary is not executable: $binary"

stage="${out}.tmp.$$"
cleanup() {
    rm -rf -- "$stage"
}
trap cleanup EXIT

rm -rf -- "$stage"
mkdir -p -- "$stage/apps" "$stage/nginx" "$stage/systemd"

install -m 0755 -- "$binary" "$stage/swapdock"
install -m 0644 -- "$repo_root/install/apps/README.md" "$stage/apps/README.md"
install -m 0644 -- "$repo_root"/install/nginx/*.conf "$stage/nginx/"
install -m 0644 -- "$repo_root"/install/nginx/*.example "$stage/nginx/"
if [[ -f "$repo_root/web/dist/index.html" ]]; then
    mkdir -p "$stage/ui"
    cp -a "$repo_root/web/dist/." "$stage/ui/"
    echo "ui:   web/dist"
else
    echo "warning: web/dist/index.html is missing; the console will not be packaged" >&2
fi
install -m 0644 -- "$repo_root/install/logrotate-swapdock" "$stage/logrotate-swapdock"
install -m 0755 -- "$repo_root/install/install-swapdock.sh" "$stage/install-swapdock.sh"
install -m 0755 -- "$repo_root/install/swapdock-root" "$stage/swapdock-root"
install -m 0755 -- "$repo_root/install/uninstall-swapdock.sh" "$stage/uninstall-swapdock.sh"
install -m 0644 -- "$repo_root/LICENSE-MIT" "$stage/LICENSE-MIT"
if [[ -f "$repo_root/install/systemd/swapdock.service" ]]; then
    install -m 0644 -- "$repo_root/install/systemd/swapdock.service" "$stage/systemd/swapdock.service"
fi
if [[ -f "$repo_root/install/systemd/swapdock-nonroot.service" ]]; then
    install -m 0644 -- "$repo_root/install/systemd/swapdock-nonroot.service" "$stage/systemd/swapdock-nonroot.service"
fi
if [[ -f "$repo_root/install/systemd/swapdock-broker.service" ]]; then
    install -m 0644 -- "$repo_root/install/systemd/swapdock-broker.service" "$stage/systemd/swapdock-broker.service"
fi
if [[ -f "$repo_root/install/systemd/swapdock-backup.service" ]]; then
    install -m 0644 -- "$repo_root/install/systemd/swapdock-backup.service" "$stage/systemd/swapdock-backup.service"
    install -m 0644 -- "$repo_root/install/systemd/swapdock-backup.timer" "$stage/systemd/swapdock-backup.timer"
fi
if [[ -f "$repo_root/install/systemd/swapdock-monitor.service" ]]; then
    install -m 0644 -- "$repo_root/install/systemd/swapdock-monitor.service" "$stage/systemd/swapdock-monitor.service"
    install -m 0644 -- "$repo_root/install/systemd/swapdock-monitor.timer" "$stage/systemd/swapdock-monitor.timer"
fi

rm -rf -- "$out"
mv -- "$stage" "$out"
trap - EXIT

printf '\npackage: %s\n' "$out"
if command -v file >/dev/null 2>&1; then
    printf 'binary:  %s\n' "$(file -b "$out/swapdock")"
fi
printf 'sha256:  %s\n\n' "$(sha256sum "$out/swapdock" | cut -d ' ' -f 1)"
printf 'contents:\n'
(cd "$out" && find . -type f | sort | sed 's|^\./|  |')
