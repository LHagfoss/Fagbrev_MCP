#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/release.sh check
  scripts/release.sh package --target TARGET --version VERSION [--dry-run]

Commands:
  check    Run the local formatting, test, lint, and release-build checks.
  package  Package a previously built target release binary into dist/.

VERSION may be a Cargo version (0.1.0) or a release tag (v0.1.0).
EOF
}

die() {
  echo "release.sh: $*" >&2
  exit 1
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ $# -eq 0 ]]; then
  usage
  exit 2
fi

command="$1"
shift

case "$command" in
  check)
    [[ $# -eq 0 ]] || die "check does not accept arguments"
    cargo fmt --all -- --check
    cargo test --locked
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cargo build --locked --release
    ;;

  package)
    target=""
    version=""
    dry_run=false

    while [[ $# -gt 0 ]]; do
      case "$1" in
        --target)
          [[ $# -ge 2 ]] || die "--target needs a value"
          target="$2"
          shift 2
          ;;
        --version)
          [[ $# -ge 2 ]] || die "--version needs a value"
          version="$2"
          shift 2
          ;;
        --dry-run)
          dry_run=true
          shift
          ;;
        *)
          die "unknown package argument: $1"
          ;;
      esac
    done

    [[ "$target" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid target"
    [[ "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || die "invalid version"

    version="${version#v}"
    binary_name="fagbrev-mcp"
    if [[ "$target" == *windows* ]]; then
      binary_name="${binary_name}.exe"
    fi

    binary_path="$repo_root/target/$target/release/$binary_name"
    archive="$repo_root/dist/fagbrev-mcp-${version}-${target}.tar.gz"

    if [[ "$dry_run" == true ]]; then
      echo "Would package $binary_path as $archive"
      exit 0
    fi

    [[ -f "$binary_path" ]] || die "release binary not found: $binary_path"
    mkdir -p "$repo_root/dist"
    [[ ! -e "$archive" ]] || die "archive already exists: $archive"

    staging_dir="$(mktemp -d "$repo_root/dist/.staging.XXXXXX")"
    trap 'rm -rf "$staging_dir"' EXIT
    cp "$binary_path" "$staging_dir/$binary_name"
    cp README.md AGENTS.md "$staging_dir/"
    tar -C "$staging_dir" -czf "$archive" "$binary_name" README.md AGENTS.md
    echo "Created $archive"
    ;;

  -h|--help)
    usage
    ;;

  *)
    die "unknown command: $command"
    ;;
esac
