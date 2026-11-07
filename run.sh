#!/usr/bin/env bash
# Astralis dev helper.
#
#   ./run.sh app          Launch the desktop app in dev mode (hot reload)
#   ./run.sh test         Run the core test suite (default)
#   ./run.sh build        Build release installers
#   ./run.sh ui           Run just the frontend in a browser (with a mock backend)
#   ./run.sh fetch-model  Download the in-app AI model (~90 MB, one time)
#   ./run.sh check|clippy|fmt|clean
#
# Extra arguments are passed through to cargo / tauri.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="$ROOT/apps/desktop"
MODEL_DIR="$APP/src-tauri/resources/models/all-MiniLM-L6-v2"
cd "$ROOT"

# This machine has gcc but not the `x86_64-linux-gnu-gcc` name cargo asks for.
if ! command -v x86_64-linux-gnu-gcc >/dev/null 2>&1 && command -v gcc >/dev/null 2>&1; then
    export CC="${CC:-gcc}"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-gcc}"
fi

# A webkit2gtk built from source into a custom prefix (default ~/.local/webkit) needs to be
# on pkg-config's path for building and on the library path for running.
WEBKIT_PREFIX="${WEBKIT_PREFIX:-$HOME/.local/webkit}"
if [[ -f "$WEBKIT_PREFIX/lib/pkgconfig/webkit2gtk-4.1.pc" ]]; then
    export PKG_CONFIG_PATH="$WEBKIT_PREFIX/lib/pkgconfig:$WEBKIT_PREFIX/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export LD_LIBRARY_PATH="$WEBKIT_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export XDG_DATA_DIRS="$WEBKIT_PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
fi

need_webkit() {
    if [[ "$(uname)" == "Linux" ]] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
        echo "The desktop app needs webkit2gtk-4.1. On Arch:" >&2
        echo "  sudo pacman -S --needed webkit2gtk-4.1 base-devel" >&2
        exit 1
    fi
}

need_npm() {
    [[ -d "$APP/node_modules" ]] || (cd "$APP" && npm install)
}

cmd="${1:-test}"
[ $# -gt 0 ] && shift

case "$cmd" in
    app)    need_webkit; need_npm; cd "$APP" && exec npx tauri dev "$@" ;;
    build)  need_webkit; need_npm; cd "$APP" && exec npx tauri build "$@" ;;
    ui)     need_npm; cd "$APP" && exec npm run dev -- --open "$@" ;;
    test)   exec cargo test "$@" ;;
    check)  exec cargo check --all-targets "$@" ;;
    clippy) exec cargo clippy --all-targets "$@" ;;
    fmt)    exec cargo fmt --all "$@" ;;
    clean)  exec cargo clean "$@" ;;
    fetch-model)
        # sentence-transformers/all-MiniLM-L6-v2 (Apache-2.0), from Hugging Face.
        base="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
        mkdir -p "$MODEL_DIR"
        for f in config.json tokenizer.json model.safetensors; do
            if [[ -s "$MODEL_DIR/$f" ]]; then echo "✓ $f"; continue; fi
            echo "↓ $f"
            curl -fL --progress-bar "$base/$f" -o "$MODEL_DIR/$f.part" && mv "$MODEL_DIR/$f.part" "$MODEL_DIR/$f"
        done
        echo "Model ready in $MODEL_DIR"
        ;;
    *) echo "Unknown command: $cmd (use app|test|build|ui|fetch-model|check|clippy|fmt|clean)" >&2; exit 1 ;;
esac
